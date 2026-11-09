//! Lists the game's voice events and what each audio table says for them.
//!
//!   cargo run -p rc_combat --example rc_audio_tables -- <game folder> [Table ...]
//!
//! With no table names it prints every event name of EPawnAudioEvent (from
//! Engine.u). With names (like Delta07Audio) it prints, for each table, the
//! events that have a sound group, one per line: `Delta07Audio  FormUp  D07_FormUp`.
//! Reads the install only; writes nothing.

use rc_combat::audio_table::{read_audio_events, read_audio_table};
use rc_game_core::GamePaths;

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(game) = args.next() else {
        eprintln!("usage: rc_audio_tables <game folder> [Table ...]");
        std::process::exit(2);
    };
    let paths = GamePaths::new(game, Vec::new());
    let events = match read_audio_events(&paths) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("can't read events: {e}");
            std::process::exit(1);
        }
    };
    let tables: Vec<String> = args.collect();
    if tables.is_empty() {
        for (i, e) in events.iter().enumerate() {
            println!("{i}\t{e}");
        }
        return;
    }
    for t in &tables {
        match read_audio_table(&paths, t, &events) {
            Ok(rows) => {
                // The game names each group <prefix>_<event>, so a group
                // whose name doesn't end in its event means the list is
                // being read wrongly.
                let odd = rows
                    .iter()
                    .filter(|(e, g)| {
                        !g.to_ascii_lowercase()
                            .ends_with(&format!("_{}", e.to_ascii_lowercase()))
                    })
                    .count();
                println!(
                    "# {t}: {} events with lines, {odd} with a group named for another event",
                    rows.len()
                );
                for (event, group) in rows {
                    println!("{t}\t{event}\t{group}");
                }
            }
            Err(e) => println!("# {t}: {e}"),
        }
    }
}
