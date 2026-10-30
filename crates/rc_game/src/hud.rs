//! PLACEHOLDER: our own stand-in for the original's HUD. When the install's
//! HUD pictures can be read, `original_hud.rs` draws the original's
//! pictures instead and hides these drawings (only the crosshair bars are
//! still used, as a fallback); see `docs/ORIGINAL_MENUS.md`.
//!
//! The on-screen display during play, styled after the commando's visor:
//! amber corner brackets, the crosshair in the middle, shield and health at
//! bottom left, ammunition at bottom right, the squad at top left, and short
//! messages at the top.
//!
//! It only reads `PlayerVitals`, `WeaponReadout`, `SquadReadout` and
//! `HudMessage`, so gameplay can change without touching it.

use crate::settings::Settings;
use crate::theme::Theme;
use bevy::prelude::*;
use rc_game_core::{
    GameLog, GameSet, GameState, HudMessage, PlayerVitals, SquadReadout, SquadStatus, WeaponReadout,
};

/// Seconds between display lines in the log.
const LOG_EVERY: f32 = 2.0;
/// Length of each crosshair bar, % of screen height.
const CROSS_LEN: f32 = 1.6;

pub struct HudPlugin;

impl Plugin for HudPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<HudLogTimer>()
            .add_systems(OnEnter(GameState::InGame), spawn_hud)
            .add_systems(
                Update,
                (
                    update_bars,
                    update_crosshair,
                    update_weapon,
                    update_squad,
                    update_messages,
                    log_hud,
                )
                    .in_set(GameSet::Hud)
                    .in_set(HudSystems)
                    .run_if(in_state(GameState::InGame)),
            );
    }
}

#[derive(Component)]
struct ShieldFill;
#[derive(Component)]
struct HealthFill;
#[derive(Component)]
pub(crate) struct CrossBar(pub(crate) Vec2);

/// The placeholder's own drawings (everything but the crosshair), hidden
/// while the original's HUD pictures are drawn (`original_hud.rs`).
#[derive(Component)]
pub(crate) struct PlaceholderPart;

/// The HUD's update systems (the original HUD pictures run after them).
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct HudSystems;
#[derive(Component)]
struct WeaponName;
#[derive(Component)]
struct AmmoText;
#[derive(Component)]
struct GrenadeText;
#[derive(Component)]
struct SquadPanel;
#[derive(Component)]
struct MessageList;
#[derive(Component)]
struct Message(f32);

#[derive(Resource, Default)]
struct HudLogTimer(f32);

fn bar(
    parent: &mut ChildSpawnerCommands,
    label: &str,
    colour: Color,
    fill: impl Bundle,
    theme: &Theme,
) {
    parent
        .spawn(Node {
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            column_gap: Val::VMin(1.0),
            ..default()
        })
        .with_children(|row| {
            row.spawn((
                Text::new(label),
                theme.font(theme.small),
                TextColor(theme.text_dim),
                Node {
                    width: Val::VMin(10.0),
                    ..default()
                },
            ));
            row.spawn((
                Node {
                    width: Val::VMin(22.0),
                    height: Val::VMin(1.4),
                    border: UiRect::all(Val::Px(1.0)),
                    padding: UiRect::all(Val::Px(1.0)),
                    ..default()
                },
                BorderColor::all(theme.text_dim),
                BackgroundColor(theme.panel),
            ))
            .with_children(|b| {
                b.spawn((
                    fill,
                    Node {
                        width: Val::Percent(100.0),
                        height: Val::Percent(100.0),
                        ..default()
                    },
                    BackgroundColor(colour),
                ));
            });
        });
}

/// One L-shaped visor bracket in a corner.
fn bracket(parent: &mut ChildSpawnerCommands, theme: &Theme, top: bool, left: bool) {
    let mut node = Node {
        position_type: PositionType::Absolute,
        width: Val::VMin(6.0),
        height: Val::VMin(6.0),
        border: UiRect {
            left: Val::Px(if left { 2.0 } else { 0.0 }),
            right: Val::Px(if left { 0.0 } else { 2.0 }),
            top: Val::Px(if top { 2.0 } else { 0.0 }),
            bottom: Val::Px(if top { 0.0 } else { 2.0 }),
        },
        ..default()
    };
    let m = Val::VMin(2.5);
    if top {
        node.top = m;
    } else {
        node.bottom = m;
    }
    if left {
        node.left = m;
    } else {
        node.right = m;
    }
    parent.spawn((
        PlaceholderPart,
        node,
        BorderColor::all(theme.text.with_alpha(0.6)),
    ));
}

fn spawn_hud(
    mut commands: Commands,
    theme: Res<Theme>,
    settings: Res<Settings>,
    log: Res<GameLog>,
) {
    // Everything is placed from the edges of this area and sized by the
    // screen's height (or width, if narrower), so it fits any shape. On a
    // screen wider than `hud_max_aspect` the area is centred and no wider.
    let max = settings.video.hud_max_aspect;
    commands
        .spawn((
            Node {
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                max_width: if max > 0.0 {
                    Val::Vh(100.0 * max)
                } else {
                    Val::Auto
                },
                margin: UiRect::horizontal(Val::Auto),
                ..default()
            },
            DespawnOnExit(GameState::InGame),
            GlobalZIndex(1),
        ))
        .with_children(|root| {
            for (top, left) in [(true, true), (true, false), (false, true), (false, false)] {
                bracket(root, &theme, top, left);
            }
            // Crosshair: four short bars around a point in the middle.
            root.spawn(Node {
                position_type: PositionType::Absolute,
                left: Val::Percent(50.0),
                top: Val::Percent(50.0),
                width: Val::Px(0.0),
                height: Val::Px(0.0),
                ..default()
            })
            .with_children(|c| {
                for dir in [Vec2::X, -Vec2::X, Vec2::Y, -Vec2::Y] {
                    let (w, h) = if dir.x != 0.0 {
                        (Val::VMin(CROSS_LEN), Val::Px(2.0))
                    } else {
                        (Val::Px(2.0), Val::VMin(CROSS_LEN))
                    };
                    c.spawn((
                        CrossBar(dir),
                        Node {
                            position_type: PositionType::Absolute,
                            width: w,
                            height: h,
                            ..default()
                        },
                        BackgroundColor(theme.crosshair),
                    ));
                }
            });
            // Shield and health, bottom left.
            root.spawn((
                PlaceholderPart,
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::VMin(5.0),
                    bottom: Val::VMin(5.0),
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::VMin(0.8),
                    ..default()
                },
            ))
            .with_children(|p| {
                bar(p, "SHIELD", theme.shield, ShieldFill, &theme);
                bar(p, "HEALTH", theme.health, HealthFill, &theme);
            });
            // Weapon and ammunition, bottom right.
            root.spawn((
                PlaceholderPart,
                Node {
                    position_type: PositionType::Absolute,
                    right: Val::VMin(5.0),
                    bottom: Val::VMin(5.0),
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::FlexEnd,
                    ..default()
                },
            ))
            .with_children(|p| {
                p.spawn((
                    WeaponName,
                    Text::new(""),
                    theme.font(theme.small),
                    TextColor(theme.text_dim),
                ));
                p.spawn((
                    AmmoText,
                    Text::new(""),
                    theme.font(theme.hud_number),
                    TextColor(theme.text),
                ));
                p.spawn((
                    GrenadeText,
                    Text::new(""),
                    theme.font(theme.small),
                    TextColor(theme.text_dim),
                ));
            });
            // Squad, top left.
            root.spawn((
                PlaceholderPart,
                SquadPanel,
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::VMin(5.0),
                    top: Val::VMin(5.0),
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::VMin(0.6),
                    ..default()
                },
            ));
            // Messages, top middle.
            root.spawn((
                PlaceholderPart,
                MessageList,
                Node {
                    position_type: PositionType::Absolute,
                    top: Val::VMin(5.0),
                    width: Val::Percent(100.0),
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    ..default()
                },
            ));
        });
    log.line("hud: shown");
}

fn fraction(v: f32, max: f32) -> f32 {
    if max > 0.0 {
        (v / max).clamp(0.0, 1.0)
    } else {
        0.0
    }
}

fn update_bars(
    vitals: Res<PlayerVitals>,
    mut shield: Query<&mut Node, (With<ShieldFill>, Without<HealthFill>)>,
    mut health: Query<&mut Node, (With<HealthFill>, Without<ShieldFill>)>,
) {
    if let Ok(mut n) = shield.single_mut() {
        n.width = Val::Percent(100.0 * fraction(vitals.shield, vitals.max_shield));
    }
    if let Ok(mut n) = health.single_mut() {
        n.width = Val::Percent(100.0 * fraction(vitals.health, vitals.max_health));
    }
}

fn update_crosshair(
    weapon: Res<WeaponReadout>,
    theme: Res<Theme>,
    mut bars: Query<(&CrossBar, &mut Node, &mut BackgroundColor)>,
) {
    // Gap from the middle grows with the weapon's spread. Screen Y is down.
    let gap = 1.0 + 3.0 * weapon.spread.clamp(0.0, 1.0);
    let colour = if weapon.on_target {
        theme.crosshair_on_target
    } else {
        theme.crosshair
    };
    for (b, mut n, mut c) in &mut bars {
        c.0 = colour;
        let near = if b.0.x + b.0.y > 0.0 {
            gap
        } else {
            -(gap + CROSS_LEN)
        };
        if b.0.x != 0.0 {
            n.left = Val::VMin(near);
            n.top = Val::Px(-1.0);
        } else {
            n.top = Val::VMin(near);
            n.left = Val::Px(-1.0);
        }
    }
}

fn update_weapon(
    weapon: Res<WeaponReadout>,
    mut name: Query<&mut Text, (With<WeaponName>, Without<AmmoText>, Without<GrenadeText>)>,
    mut ammo: Query<&mut Text, (With<AmmoText>, Without<WeaponName>, Without<GrenadeText>)>,
    mut gren: Query<&mut Text, (With<GrenadeText>, Without<WeaponName>, Without<AmmoText>)>,
) {
    if !weapon.is_changed() {
        return;
    }
    if let Ok(mut t) = name.single_mut() {
        t.0 = weapon.name.to_uppercase();
    }
    if let Ok(mut t) = ammo.single_mut() {
        t.0 = ammo_text(&weapon);
    }
    if let Ok(mut t) = gren.single_mut() {
        t.0 = match &weapon.grenade {
            Some((n, c)) => format!("{} x{c}", n.to_uppercase()),
            None => String::new(),
        };
    }
}

/// "45 | 180", "45 | --" for unlimited spare ammo, "--" for no magazine.
pub fn ammo_text(w: &WeaponReadout) -> String {
    if w.name.is_empty() {
        return String::new();
    }
    let clip = w.clip.map(|c| c.to_string()).unwrap_or_else(|| "--".into());
    match (w.clip, w.reserve) {
        (None, None) => "--".into(),
        (_, Some(r)) => format!("{clip} | {r}"),
        (Some(_), None) => format!("{clip} | --"),
    }
}

fn update_squad(
    mut commands: Commands,
    squad: Res<SquadReadout>,
    theme: Res<Theme>,
    panel: Query<Entity, With<SquadPanel>>,
) {
    if !squad.is_changed() {
        return;
    }
    let Ok(panel) = panel.single() else {
        return;
    };
    commands.entity(panel).despawn_related::<Children>();
    commands.entity(panel).with_children(|p| {
        for m in &squad.members {
            let down = m.status == SquadStatus::Incapacitated;
            p.spawn(Node {
                flex_direction: FlexDirection::Row,
                column_gap: Val::VMin(1.0),
                align_items: AlignItems::Center,
                ..default()
            })
            .with_children(|row| {
                row.spawn((
                    Text::new(format!("{} {}", m.number, m.name.to_uppercase())),
                    theme.font(theme.small),
                    TextColor(if down { theme.squad_down } else { theme.text }),
                    Node {
                        width: Val::VMin(14.0),
                        ..default()
                    },
                ));
                row.spawn((
                    Node {
                        width: Val::VMin(10.0),
                        height: Val::VMin(0.9),
                        ..default()
                    },
                    BackgroundColor(theme.panel),
                ))
                .with_children(|b| {
                    b.spawn((
                        Node {
                            width: Val::Percent(100.0 * m.health.clamp(0.0, 1.0)),
                            height: Val::Percent(100.0),
                            ..default()
                        },
                        BackgroundColor(if down {
                            theme.squad_down
                        } else {
                            theme.squad_ok
                        }),
                    ));
                });
                row.spawn((
                    Text::new(if down {
                        "DOWN".to_string()
                    } else {
                        m.order.to_uppercase()
                    }),
                    theme.font(theme.small),
                    TextColor(theme.text_dim),
                ));
            });
        }
    });
}

fn update_messages(
    mut commands: Commands,
    mut incoming: MessageReader<HudMessage>,
    time: Res<Time>,
    theme: Res<Theme>,
    list: Query<Entity, With<MessageList>>,
    mut shown: Query<(Entity, &mut Message)>,
    log: Res<GameLog>,
) {
    let dt = time.delta_secs();
    for (e, mut m) in &mut shown {
        m.0 -= dt;
        if m.0 <= 0.0 {
            commands.entity(e).despawn();
        }
    }
    let Ok(list) = list.single() else {
        return;
    };
    for msg in incoming.read() {
        log.line(format!(
            "hud: message \"{}\" for {:.1} s",
            msg.text, msg.seconds
        ));
        commands.entity(list).with_children(|p| {
            p.spawn((
                Message(msg.seconds),
                Text::new(msg.text.clone()),
                theme.font(theme.small),
                TextColor(theme.text),
            ));
        });
    }
}

fn log_hud(
    time: Res<Time>,
    mut timer: ResMut<HudLogTimer>,
    vitals: Res<PlayerVitals>,
    weapon: Res<WeaponReadout>,
    squad: Res<SquadReadout>,
    log: Res<GameLog>,
) {
    timer.0 -= time.delta_secs();
    if timer.0 > 0.0 {
        return;
    }
    timer.0 = LOG_EVERY;
    log.line(format!(
        "hud: shield {:.0}/{:.0} health {:.0}/{:.0} weapon '{}' ammo '{}' squad {}",
        vitals.shield,
        vitals.max_shield,
        vitals.health,
        vitals.max_health,
        weapon.name,
        ammo_text(&weapon),
        squad
            .members
            .iter()
            .map(|m| format!("{}:{:.0}%", m.name, m.health * 100.0))
            .collect::<Vec<_>>()
            .join(" ")
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ammo_counter_text() {
        let mut w = WeaponReadout {
            name: "DC-17m".into(),
            clip: Some(60),
            clip_size: Some(60),
            reserve: Some(300),
            ..default()
        };
        assert_eq!(ammo_text(&w), "60 | 300");
        w.reserve = None;
        assert_eq!(ammo_text(&w), "60 | --");
        w.clip = None;
        assert_eq!(ammo_text(&w), "--");
        w.name.clear();
        assert_eq!(ammo_text(&w), "");
    }
}
