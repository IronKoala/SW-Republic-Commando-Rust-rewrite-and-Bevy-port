# MODLOG

Running record of changes to the Star Wars: Republic Commando Rust and Bevy
rewrite. Newest entries first. Format and rules are in `MODLOG-template.md`.

## 2026-10-29: First data loader, reads game packages (rc_probe)

- **Changed:** Added a Cargo workspace (`Cargo.toml`) with two crates: `crates/rc_package` (reads Unreal Engine 2 package headers and tables, no Bevy) and `crates/rc_probe` (command-line tool that opens packages read-only and logs what it read to `logs/rc_probe.log`). Added `README.md`, `docs/DESIGN.md` and `docs/RESEARCH.md` (what exists for this game: format docs, UE Viewer, the CT headers, licenses).
- **Why:** First step of the rewrite: prove the game's data can be read from the user's install.
- **Tested how:** 7 unit tests on made-up packages built in memory (`cargo test`). Then copied 50 real packages from the Steam install into the cloud sandbox (read-only copy, outside the repo, never committed) and ran `rc_probe` on them: all 36 `.u` script packages, 3 maps, 3 texture, 3 sound, 3 static mesh and 4 animation packages.
- **Result:** 50 of 50 parsed and passed every consistency check, in 33 ms total. `core.u`: package version 159, licensee 1, 408 names, 17 imports, 809 exports; the first export's data starts exactly where the export table ends (byte 23692). Versions seen: 135, 138, 140, 145, 154, 156, 157, 158, 159, so both the standard and the Republic Commando export layouts were exercised. `vehicles.ukx`, which an old UModel forum thread said failed, reads fine.
- **Still broken / not tested:** Not tested on the user's Windows PC (run there by the cloud sandbox only). Not tested: the other 223 packages in the install. Object contents (textures, meshes, sounds, levels) are not decoded yet. The extra export field from version 159 has unknown meaning.
- **Next:** Run the full scan on the user's PC, then decode one object type (probably textures or the script text).

## 2026-10-29: Switch .gitignore to a whitelist

- **Changed:** Replaced `.gitignore` (GitHub's standard Rust list) with a whitelist. It ignores everything, then allows only `.gitignore`, `Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml`, `*.rs` and `*.md`. It never looks inside `target/`, `extracted/` or `game-data/`.
- **Why:** Project rule 2 says `.gitignore` must be a whitelist, so game files can never be committed by accident.
- **Tested how:** Made a throwaway git repo, copied in this `.gitignore`, created fake files of each kind, ran `git add -A` and checked what git picked up.
- **Result:** Tracked only `.gitignore`, `Cargo.toml`, `Cargo.lock`, `README.md`, `MODLOG.md`, `docs/DESIGN.md`, `src/main.rs`. Ignored `target/debug/build/gen.rs`, `extracted/level.rs`, `extracted/notes.md`, `game-data/Map.ctm`, `assets/clone.utx`, `tex.dds`, `save.sav`, `run.log`, `secrets.env`.
- **Still broken / not tested:** Not tested in a real Rust project yet. New kinds of source files (for example shader files) will need their own line added.
- **Next:** Set up the empty Rust and Bevy project.

## 2026-10-29: Project setup and MODLOG started

- **Changed:** Created `MODLOG.md` (this file) and `MODLOG-template.md`. No code yet.
- **Why:** The project rules require a log entry after every change, so the log has to exist before any work starts.
- **Tested how:** Not applicable. These are text files only. Nothing was run.
- **Result:** Both files added to the GitHub repository `IronKoala/SW-Republic-Commando-Rust-rewrite-and-Bevy-port` on the branch `add-modlog`. Not committed yet.
- **Still broken / not tested:** Not tested: nothing to run yet. `README.md` and `docs/DESIGN.md` do not exist yet. The repository's `.gitignore` is GitHub's standard Rust ignore list, not the whitelist the project rules require.
- **Next:** Replace `.gitignore` with a whitelist, then set up the empty Rust and Bevy project.
