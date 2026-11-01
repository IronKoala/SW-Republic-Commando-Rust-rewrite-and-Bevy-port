//! The ammo counter on the first-person gun: the DC-17m's little screen.
//!
//! In the game the screen is part of the arms model. Its pieces use
//! `TexPanner2D` materials, which slide one picture (`DC17mAmmo`) sideways:
//!
//! - Three digit pieces (the ammo class's `PannerDigit100`, `PannerDigit10`,
//!   `PannerDigit1`) each show one cell of a strip of 11 (blank, then 0 to
//!   9). They show the ammo carried in all, magazine included (the game's
//!   `AmmoAmount`), so the DC-17m starts at "120" and a full load is "300".
//! - A bar piece (`PannerCharge`) shows the right half of a row of ticks
//!   and slides up to half the picture's width to the left as the magazine
//!   empties, so the ticks go out one side.
//!
//! What we read from the game's engine (`Engine.dll`, by its numbers, not
//! its code): each digit's `OffsetU` is `digit / 11 + 2 / 11`, the
//! hundreds are the count divided by 100, there is no blank leading zero,
//! and the bar's `OffsetU` is `-0.5 * (1 - shots in magazine / magazine
//! size)`. The picture repeats, so the digit shift is the same as
//! `(digit - 9) / 11`, which we use: it stays inside the picture, and Bevy
//! doesn't have to repeat it.
//!
//! Plain rules, no Bevy; `arms_view` moves the pictures.

use crate::weapon::{WeaponParams, WeaponState};

/// Cells in the digit strip: blank, then 0 to 9.
pub const DIGIT_CELLS: f32 = 11.0;
/// How far the bar slides when the magazine is empty (picture widths).
pub const BAR_TRAVEL: f32 = 0.5;

/// What the gun's screen shows.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CounterReading {
    /// Hundreds, tens and ones of the ammo carried.
    pub digits: [u32; 3],
    /// How full the magazine bar is, 0 to 1.
    pub bar: f32,
}

impl CounterReading {
    /// For `p` in the state `w`. While reloading, the bar fills toward the
    /// new magazine over the reload time (the game animates it too); the
    /// digits don't change, as no ammo is used.
    pub fn of(p: &WeaponParams, w: &WeaponState) -> CounterReading {
        let in_clip = w.shots_in_clip();
        let total = in_clip + w.reserve;
        let digits = [(total / 100).min(9), total / 10 % 10, total % 10];
        // Recharging weapons: the charge against the most it holds (our
        // guess; the game reads other numbers there, not looked into).
        let size = if p.recharges() {
            p.max_ammo
        } else {
            p.clip_size
        };
        let mut shots = if p.recharges() {
            w.clip
        } else {
            in_clip as f32
        };
        if let Some(left) = w.reloading {
            let after = p.clip_size.min(total) as f32;
            let done = if p.reload_time > 0.0 {
                (1.0 - left / p.reload_time).clamp(0.0, 1.0)
            } else {
                1.0
            };
            shots = (shots + (after - shots) * done).floor();
        }
        let bar = if size == 0 {
            0.0
        } else {
            (shots / size as f32).clamp(0.0, 1.0)
        };
        CounterReading { digits, bar }
    }

    /// The digits as text, like "110".
    pub fn text(&self) -> String {
        self.digits.iter().map(|d| d.to_string()).collect()
    }
}

/// How far to slide a digit piece's picture (picture widths, along U) to
/// show `digit`. The pieces show the strip's last cell ("9") unslid.
pub fn digit_shift(digit: u32) -> f32 {
    (digit.min(9) as f32 - 9.0) / DIGIT_CELLS
}

/// The digit a slide of `shift` shows (for checks): the inverse of
/// [`digit_shift`].
pub fn digit_of_shift(shift: f32) -> u32 {
    (shift * DIGIT_CELLS + 9.0).round().clamp(0.0, 9.0) as u32
}

/// How far to slide the bar piece's picture for a bar `fill` (0 to 1).
pub fn bar_shift(fill: f32) -> f32 {
    -BAR_TRAVEL * (1.0 - fill.clamp(0.0, 1.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dc17m() -> WeaponParams {
        let text = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../data/weapons/dc17m_blaster.toml"
        ))
        .unwrap();
        WeaponParams::from_text("dc17m_blaster", &text).unwrap()
    }

    #[test]
    fn starts_at_the_ammo_carried_with_a_full_bar() {
        let p = dc17m();
        let w = WeaponState::new(&p);
        let r = CounterReading::of(&p, &w);
        assert_eq!(r.text(), "120");
        assert_eq!(r.bar, 1.0);
    }

    #[test]
    fn counts_down_and_the_bar_empties() {
        let p = dc17m();
        let mut w = WeaponState::new(&p);
        w.clip = 50.0;
        let r = CounterReading::of(&p, &w);
        assert_eq!(r.text(), "110");
        assert!((r.bar - 50.0 / 60.0).abs() < 1e-6);
        w.clip = 0.0;
        w.reserve = 7;
        let r = CounterReading::of(&p, &w);
        assert_eq!(r.digits, [0, 0, 7]);
        assert_eq!(r.bar, 0.0);
    }

    #[test]
    fn the_bar_refills_during_a_reload() {
        let p = dc17m();
        let mut w = WeaponState::new(&p);
        w.clip = 0.0;
        w.reserve = 60;
        assert!(w.start_reload(&p));
        assert_eq!(CounterReading::of(&p, &w).bar, 0.0);
        w.reloading = Some(p.reload_time / 2.0);
        let half = CounterReading::of(&p, &w);
        assert!((half.bar - 0.5).abs() < 1e-6, "{half:?}");
        assert_eq!(half.text(), "060");
    }

    #[test]
    fn shifts_pick_the_right_cell() {
        // The pieces' pictures sit on the strip's last cell, 0.902..0.991
        // across (the DC-17m's arms model); the middle of the slid piece
        // must land in the digit's own cell (cell 0 is the blank).
        let middle = (0.902 + 0.991) / 2.0;
        for d in 0..10 {
            let u: f32 = middle + digit_shift(d);
            let cell = (u * DIGIT_CELLS).floor() as u32;
            assert_eq!(cell, d + 1, "digit {d} lands in cell {cell}");
            assert_eq!(digit_of_shift(digit_shift(d)), d);
            // The game's own offset, d/11 + 2/11, is the same once the
            // picture repeats.
            let game = d as f32 / DIGIT_CELLS + 2.0 / DIGIT_CELLS;
            assert!((game - digit_shift(d) - 1.0).abs() < 1e-6);
        }
        assert_eq!(bar_shift(1.0), 0.0);
        assert_eq!(bar_shift(0.0), -0.5);
    }

    #[test]
    fn more_than_999_shows_9_hundreds() {
        let p = dc17m();
        let mut w = WeaponState::new(&p);
        w.clip = 60.0;
        w.reserve = 1200;
        assert_eq!(CounterReading::of(&p, &w).digits, [9, 6, 0]);
    }
}
