//! One log file for the whole game, `logs/rc_game.log`.
//!
//! "Log, don't look": every plugin writes what it did (counts, positions,
//! timings, state changes) here, so a run can be checked from the numbers.

use bevy::prelude::Resource;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Instant;

#[derive(Resource, Clone)]
pub struct GameLog {
    file: Arc<Mutex<Option<fs::File>>>,
    start: Instant,
}

impl GameLog {
    /// Log to `path` (its folder is created), and to the terminal.
    pub fn create(path: &Path) -> std::io::Result<Self> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let f = fs::File::create(path)?;
        Ok(GameLog {
            file: Arc::new(Mutex::new(Some(f))),
            start: Instant::now(),
        })
    }

    /// Log to the terminal only (for tests).
    pub fn terminal() -> Self {
        GameLog {
            file: Arc::new(Mutex::new(None)),
            start: Instant::now(),
        }
    }

    /// Write one line, stamped with seconds since start. Start the line with
    /// your part's name, like `"weapons: fired dc15s"`.
    pub fn line(&self, s: impl AsRef<str>) {
        let s = format!(
            "[{:>8.3}s] {}",
            self.start.elapsed().as_secs_f64(),
            s.as_ref()
        );
        println!("{s}");
        if let Ok(mut f) = self.file.lock() {
            if let Some(f) = f.as_mut() {
                let _ = writeln!(f, "{s}");
            }
        }
    }
}
