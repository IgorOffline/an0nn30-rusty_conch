//! Cross-platform menu bar.
//!
//! On macOS: native NSMenu global menu bar.
//! On other platforms: egui in-window menu bar.
//!
//! Designed for extensibility: plugins will register additional items
//! via `MenuBarState` in a future phase.

#[cfg(target_os = "macos")]
mod native_macos;

mod egui_menu;

use egui::ViewportCommand;

use crate::platform::PlatformCapabilities;

/// Actions that menu items can trigger.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MenuAction {
    // File
    NewTab,
    NewWindow,
    CloseTab,
    Quit,
    // Edit
    Copy,
    Paste,
    SelectAll,
    // View
    ZenMode,
    ZoomIn,
    ZoomOut,
    ZoomReset,
}

/// Persistent menu bar state. Plugins will register items here in Phase 2.
#[derive(Default)]
pub struct MenuBarState {
    /// Whether the native menu has been set up (macOS only, done once).
    native_setup_done: bool,
}

/// Set up the menu bar. On macOS, installs the native NSMenu (once).
/// On other platforms, this is a no-op (egui menu renders each frame).
pub fn setup(state: &mut MenuBarState, _platform: &PlatformCapabilities) {
    if state.native_setup_done {
        return;
    }

    #[cfg(target_os = "macos")]
    {
        if _platform.native_global_menu {
            native_macos::setup_menu_bar();
            state.native_setup_done = true;
            return;
        }
    }

    state.native_setup_done = true;
}

/// Render the menu bar (if in-window) and collect any triggered actions.
pub fn show(
    ctx: &egui::Context,
    state: &mut MenuBarState,
    platform: &PlatformCapabilities,
) -> Option<MenuAction> {
    // Ensure native menu is set up on first frame.
    setup(state, platform);

    // On macOS with native menu, drain actions from the ObjC channel.
    #[cfg(target_os = "macos")]
    if platform.native_global_menu {
        return native_macos::drain_actions().into_iter().next();
    }

    // Fallback: egui in-window menu bar.
    egui_menu::show(ctx)
}

/// Handle a menu action, mutating app state as needed.
pub fn handle_action(
    action: MenuAction,
    ctx: &egui::Context,
    app: &mut super::app::ConchApp,
) {
    match action {
        MenuAction::NewTab => app.open_local_tab(),
        MenuAction::NewWindow => app.spawn_extra_window(),
        MenuAction::CloseTab => {
            if let Some(id) = app.state.active_tab {
                app.remove_session(id);
            }
        }
        MenuAction::Quit => {
            app.quit_requested = true;
        }
        MenuAction::Copy => {
            if let Some((start, end)) = app.selection.normalized() {
                if let Some(session) = app.state.active_session() {
                    let text = crate::terminal::widget::get_selected_text(session.term(), start, end);
                    if !text.is_empty() {
                        ctx.copy_text(text);
                    }
                }
            }
        }
        MenuAction::Paste => {
            ctx.send_viewport_cmd(ViewportCommand::RequestPaste);
        }
        MenuAction::SelectAll => {
            // TODO: implement select-all for terminal content
        }
        MenuAction::ZenMode => {
            // TODO: toggle zen mode (hide chrome)
        }
        MenuAction::ZoomIn => {
            let current = ctx.pixels_per_point();
            ctx.set_pixels_per_point(current + 0.5);
        }
        MenuAction::ZoomOut => {
            let current = ctx.pixels_per_point();
            ctx.set_pixels_per_point((current - 0.5).max(0.5));
        }
        MenuAction::ZoomReset => {
            ctx.set_pixels_per_point(1.0);
        }
    }
}
