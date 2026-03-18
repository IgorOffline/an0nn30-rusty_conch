//! File watcher for config/theme hot-reload.
//!
//! Watches config.toml and themes/ directory. On change, emits a Tauri event
//! so the frontend can re-fetch theme colors and terminal config.

use std::path::PathBuf;
use std::time::Duration;

use tauri::Emitter;

/// Start watching config and theme files. Returns a thread join handle.
pub fn start(app_handle: tauri::AppHandle) -> std::thread::JoinHandle<()> {
    std::thread::Builder::new()
        .name("config-watcher".into())
        .spawn(move || {
            watch_loop(app_handle);
        })
        .expect("Failed to spawn config watcher thread")
}

fn watch_loop(app: tauri::AppHandle) {
    use std::collections::HashMap;
    use std::fs;

    let config_path = conch_core::config::config_path();
    let themes_dir = conch_core::color_scheme::themes_dir();

    let mut mtimes: HashMap<PathBuf, std::time::SystemTime> = HashMap::new();

    // Seed initial mtimes.
    if let Ok(meta) = fs::metadata(&config_path) {
        if let Ok(mtime) = meta.modified() {
            mtimes.insert(config_path.clone(), mtime);
        }
    }
    seed_dir_mtimes(&themes_dir, &mut mtimes);

    loop {
        std::thread::sleep(Duration::from_secs(2));

        let mut changed = false;

        // Check config.toml.
        if let Ok(meta) = fs::metadata(&config_path) {
            if let Ok(mtime) = meta.modified() {
                if mtimes.get(&config_path) != Some(&mtime) {
                    mtimes.insert(config_path.clone(), mtime);
                    changed = true;
                    log::info!("Config file changed, reloading");
                }
            }
        }

        // Check themes directory.
        if themes_dir.exists() {
            if let Ok(entries) = fs::read_dir(&themes_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.extension().map_or(false, |e| e == "toml") {
                        if let Ok(meta) = fs::metadata(&path) {
                            if let Ok(mtime) = meta.modified() {
                                if mtimes.get(&path) != Some(&mtime) {
                                    mtimes.insert(path, mtime);
                                    changed = true;
                                    log::info!("Theme file changed, reloading");
                                }
                            }
                        }
                    }
                }
            }
        }

        if changed {
            let _ = app.emit("config-changed", ());
        }
    }
}

fn seed_dir_mtimes(dir: &PathBuf, mtimes: &mut std::collections::HashMap<PathBuf, std::time::SystemTime>) {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if let Ok(meta) = std::fs::metadata(&path) {
                if let Ok(mtime) = meta.modified() {
                    mtimes.insert(path, mtime);
                }
            }
        }
    }
}
