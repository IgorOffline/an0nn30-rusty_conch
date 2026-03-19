#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use conch_core::config;

fn main() {
    // Platform init MUST run before anything else — fixes locale, PATH, and
    // SSH_AUTH_SOCK when launched from Finder/desktop (not a terminal).
    conch_tauri::platform::init();

    env_logger::init();

    let user_config = config::load_user_config().unwrap_or_else(|e| {
        log::error!("Failed to load config.toml, using defaults: {e:#}");
        config::UserConfig::default()
    });

    if let Err(e) = conch_tauri::run(user_config) {
        eprintln!("Fatal error: {e}");
        std::process::exit(1);
    }
}
