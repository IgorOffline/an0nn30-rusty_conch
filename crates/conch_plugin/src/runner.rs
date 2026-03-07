use std::path::Path;

use anyhow::Result;
use mlua::Lua;

use crate::api::{self, PluginCommand, PluginContext};

/// Execute a Lua plugin script with the full API available (run-once action).
pub async fn run_plugin(path: &Path, ctx: PluginContext) -> Result<()> {
    let lua = Lua::new();

    // Sandbox: remove dangerous modules
    sandbox(&lua)?;

    // Configure package paths for LuaRocks modules and local requires
    configure_package_paths(&lua, path)?;

    // Register API tables
    api::session::register(&lua, ctx.clone())?;
    api::app::register(&lua, ctx.clone())?;
    api::ui::register(&lua, ctx)?;
    api::crypto::register(&lua)?;
    api::net::register(&lua)?;

    let script = std::fs::read_to_string(path)?;
    lua.load(&script).exec_async().await?;

    Ok(())
}

/// Execute a panel plugin: call setup(), then loop calling render() and
/// flushing the widget list to the app at the configured refresh interval.
pub async fn run_panel_plugin(path: &Path, ctx: PluginContext) -> Result<()> {
    let lua = Lua::new();

    sandbox(&lua)?;
    configure_package_paths(&lua, path)?;

    api::session::register(&lua, ctx.clone())?;
    api::app::register(&lua, ctx.clone())?;
    api::ui::register(&lua, ctx.clone())?;
    api::crypto::register(&lua)?;
    api::net::register(&lua)?;

    // Load the script (defines setup/render/on_click functions)
    let script = std::fs::read_to_string(path)?;
    lua.load(&script).exec_async().await?;

    // Call setup() if defined
    let globals = lua.globals();
    if let Ok(setup_fn) = globals.get::<mlua::Function>("setup") {
        setup_fn.call_async::<()>(()).await?;
    }

    // Default refresh interval: 10 seconds
    let refresh_secs: f64 = 10.0;

    loop {
        // Call render() if defined
        if let Ok(render_fn) = globals.get::<mlua::Function>("render") {
            render_fn.call_async::<()>(()).await?;
        }

        // Collect widgets from Lua registry and send to app
        let widgets = api::ui::collect_panel_widgets(&lua)?;
        let _ = ctx
            .send_command(PluginCommand::PanelSetWidgets(widgets))
            .await;

        // Poll for button/keybind events while waiting for the refresh interval.
        let interval = if refresh_secs > 0.0 { refresh_secs } else { 60.0 };
        let poll_interval = std::time::Duration::from_millis(150);
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs_f64(interval);

        while tokio::time::Instant::now() < deadline {
            let resp = ctx.send_command(PluginCommand::PanelPollEvent).await;
            match resp {
                api::PluginResponse::PanelEvent(button_id) => {
                    if let Ok(on_click) = globals.get::<mlua::Function>("on_click") {
                        let _ = on_click.call_async::<()>(button_id).await;
                    }
                    break; // Re-render immediately after handling event
                }
                api::PluginResponse::KeybindTriggered(action) => {
                    if let Ok(on_keybind) = globals.get::<mlua::Function>("on_keybind") {
                        let _ = on_keybind.call_async::<()>(action).await;
                    }
                    break; // Re-render immediately after handling event
                }
                _ => {
                    // No event — sleep briefly before polling again
                    tokio::time::sleep(poll_interval).await;
                }
            }
        }
    }
}

/// Remove dangerous Lua standard library functions for sandboxing.
fn sandbox(lua: &Lua) -> Result<()> {
    let globals = lua.globals();

    // Remove os module (file ops, process exec, etc.)
    globals.set("os", mlua::Value::Nil)?;
    // Remove io module (file I/O)
    globals.set("io", mlua::Value::Nil)?;
    // Remove loadfile/dofile (arbitrary file execution)
    globals.set("loadfile", mlua::Value::Nil)?;
    globals.set("dofile", mlua::Value::Nil)?;

    Ok(())
}

/// Configure `package.path` and `package.cpath` so plugins can:
/// 1. `require()` local helper files from the plugin's own directory
/// 2. `require()` LuaRocks-installed modules from `~/.config/conch/lua_modules/`
fn configure_package_paths(lua: &Lua, plugin_path: &Path) -> Result<()> {
    let luarocks_base = conch_core::config::config_dir()
        .join("lua_modules")
        .to_string_lossy()
        .into_owned();

    // Plugin's own directory for local requires
    let plugin_dir = plugin_path
        .parent()
        .unwrap_or(Path::new("."))
        .to_string_lossy();

    lua.load(format!(
        r#"
        local plugin_dir = "{plugin_dir}"
        local luarocks = "{luarocks_base}"

        package.path = plugin_dir .. "/?.lua;"
            .. plugin_dir .. "/?/init.lua;"
            .. luarocks .. "/share/lua/5.4/?.lua;"
            .. luarocks .. "/share/lua/5.4/?/init.lua;"
            .. package.path

        package.cpath = luarocks .. "/lib/lua/5.4/?.so;"
            .. luarocks .. "/lib/lua/5.4/?.dylib;"
            .. package.cpath
        "#
    ))
    .exec()?;

    Ok(())
}
