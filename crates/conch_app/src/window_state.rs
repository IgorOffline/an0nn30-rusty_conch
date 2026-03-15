//! Per-window state and shared app state for the unified window architecture.
//!
//! `WindowState` — all state unique to a single OS window.
//! `SharedAppState` — global state shared across all windows (Arc-wrapped).
//! `WindowAction` — cross-cutting actions sent from windows to the coordinator.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use conch_core::config;
use conch_plugin::bus::PluginBus;
use conch_plugin_sdk::PanelLocation;
use parking_lot::Mutex;
use uuid::Uuid;

use crate::context_menu::ContextMenuState;
use crate::host::bridge::PanelRegistry;
use crate::host::bridge::SessionRegistry;
use crate::host::dialogs::DialogState;
use crate::icons::IconCache;
use crate::input::{ResolvedPluginKeybind, ResolvedShortcuts};
use crate::menu_bar::MenuBarState;
use crate::mouse::Selection;
use crate::notifications::NotificationManager;
use crate::platform::PlatformCapabilities;
use crate::sessions::create_local_session;
use crate::state::Session;
use crate::tab_bar::TabBarState;
use crate::terminal::color::ResolvedColors;
use crate::terminal::widget::TerminalFrameCache;
use crate::ui_theme::UiTheme;

// ── SharedConfig ──

/// Read-mostly configuration state updated on config/theme reload.
pub(crate) struct SharedConfig {
    pub user_config: config::UserConfig,
    pub persistent: config::PersistentState,
    pub colors: ResolvedColors,
    pub theme: UiTheme,
    pub theme_dirty: bool,
    pub shortcuts: ResolvedShortcuts,
    pub plugin_keybindings: Vec<ResolvedPluginKeybind>,
    pub plugin_keybindings_version: u64,
}

// ── SharedAppState ──

/// Global state shared across all windows.
///
/// All fields use interior-mutability wrappers (`Mutex`) for `Send + Sync`
/// compatibility with deferred viewport closures. Since all viewports render
/// on the same thread, there is no actual lock contention.
pub(crate) struct SharedAppState {
    /// Configuration, theme, colors, shortcuts.
    pub config: Mutex<SharedConfig>,
    /// Plugin publish/subscribe event bus.
    pub plugin_bus: Arc<PluginBus>,
    /// Registered plugin panels (location + name).
    pub panel_registry: Arc<Mutex<PanelRegistry>>,
    /// Pending session open/close from plugins.
    pub session_registry: Arc<Mutex<SessionRegistry>>,
    /// Cached widget JSON per plugin name.
    pub render_cache: Mutex<HashMap<String, String>>,
    /// Plugin dialog state (per-viewport).
    pub dialog_state: Mutex<DialogState>,
    /// Toast notification manager.
    pub notifications: Mutex<NotificationManager>,
    /// Icon cache (lazy-initialized).
    pub icon_cache: Mutex<Option<IconCache>>,
    /// Menu bar rendering state.
    pub menu_bar_state: Mutex<MenuBarState>,
    /// Platform capabilities (immutable).
    pub platform: PlatformCapabilities,
}

// ── WindowAction ──

/// Actions that windows send to the coordinator (`ConchApp::update()`).
///
/// Windows can't mutate `ConchApp` directly (deferred viewport callbacks
/// are `Fn`, not `FnMut`). Cross-cutting operations go through this channel.
pub(crate) enum WindowAction {
    SpawnNewWindow,
    Quit,
    PluginAction(crate::host::plugin_manager_ui::PluginManagerAction),
    WindowClosed(egui::ViewportId),
    SavePanelSizes {
        left: Option<f32>,
        right: Option<f32>,
        bottom: Option<f32>,
    },
    PublishTabChanged {
        is_ssh: bool,
        session_id: Option<u64>,
    },
}

// ── WindowState ──

/// Per-window state shared by main and extra windows.
pub(crate) struct WindowState {
    // ── Sessions / tabs ──
    pub sessions: HashMap<Uuid, Session>,
    pub tab_order: Vec<Uuid>,
    pub active_tab: Option<Uuid>,

    // ── Terminal rendering ──
    pub cell_width: f32,
    pub cell_height: f32,
    pub cell_size_measured: bool,
    pub last_pixels_per_point: f32,
    pub last_cols: u16,
    pub last_rows: u16,
    pub cursor_visible: bool,
    pub last_blink: Instant,
    pub frame_cache: TerminalFrameCache,
    pub selection: Selection,

    // ── UI chrome ──
    pub tab_bar_state: TabBarState,
    pub context_menu_state: ContextMenuState,
    pub show_plugin_manager: bool,
    pub left_panel_visible: bool,
    pub right_panel_visible: bool,
    pub bottom_panel_visible: bool,
    pub show_status_bar: bool,
    /// Mutable text input state for plugin panels (keyed by widget id).
    pub plugin_text_state: HashMap<String, String>,
    /// Active panel tab per location (handle of the selected panel).
    pub active_panel_tab: HashMap<PanelLocation, u64>,

    // ── Viewport info ──
    pub viewport_id: egui::ViewportId,
    pub viewport_builder: Option<egui::ViewportBuilder>,
    pub title: String,
    pub should_close: bool,
    pub style_applied: bool,
    /// Whether this window had OS focus during the last frame.
    pub has_focus: bool,
    /// Pending actions to send to the coordinator.
    pub pending_actions: Vec<WindowAction>,
    /// Menu actions routed from the native menu bar.
    pub pending_menu_actions: Vec<crate::menu_bar::MenuAction>,
}

impl WindowState {
    /// Create a new window state for a given viewport.
    pub fn new(viewport_id: egui::ViewportId) -> Self {
        Self {
            sessions: HashMap::new(),
            tab_order: Vec::new(),
            active_tab: None,
            cell_width: 0.0,
            cell_height: 0.0,
            cell_size_measured: false,
            last_pixels_per_point: 0.0,
            last_cols: 0,
            last_rows: 0,
            cursor_visible: true,
            last_blink: Instant::now(),
            frame_cache: TerminalFrameCache::default(),
            selection: Selection::default(),
            tab_bar_state: TabBarState::default(),
            context_menu_state: ContextMenuState::default(),
            show_plugin_manager: false,
            left_panel_visible: true,
            right_panel_visible: true,
            bottom_panel_visible: true,
            show_status_bar: true,
            plugin_text_state: HashMap::new(),
            active_panel_tab: HashMap::new(),
            viewport_id,
            viewport_builder: None,
            title: String::new(),
            should_close: false,
            style_applied: false,
            has_focus: false,
            pending_actions: Vec::new(),
            pending_menu_actions: Vec::new(),
        }
    }

    /// Create a new window state with an initial session.
    pub fn with_session(
        viewport_id: egui::ViewportId,
        viewport_builder: egui::ViewportBuilder,
        session: Session,
    ) -> Self {
        let mut state = Self::new(viewport_id);
        let id = session.id;
        state.title = session.display_title().to_string();
        state.viewport_builder = Some(viewport_builder);
        state.sessions.insert(id, session);
        state.tab_order.push(id);
        state.active_tab = Some(id);
        state
    }

    /// Get the currently active session, if any.
    pub fn active_session(&self) -> Option<&Session> {
        self.active_tab.and_then(|id| self.sessions.get(&id))
    }

    /// Get a mutable reference to the active session.
    pub fn active_session_mut(&mut self) -> Option<&mut Session> {
        self.active_tab.and_then(|id| self.sessions.get_mut(&id))
    }

    /// Open a new local terminal tab, inheriting the CWD from the active session.
    pub fn open_local_tab(&mut self, user_config: &config::UserConfig) {
        let cwd = self.active_tab
            .and_then(|id| self.sessions.get(&id))
            .and_then(|s| s.child_pid())
            .and_then(conch_pty::get_cwd_of_pid);
        if let Some((id, session)) = create_local_session(user_config, cwd) {
            if self.last_cols > 0 && self.last_rows > 0 {
                session.resize(
                    self.last_cols, self.last_rows,
                    self.cell_width as u16, self.cell_height as u16,
                );
            }
            self.sessions.insert(id, session);
            self.tab_order.push(id);
            self.active_tab = Some(id);
        }
    }

    /// Remove a session by ID, triggering the close animation.
    pub fn remove_session(&mut self, id: Uuid) {
        let title = self.sessions.get(&id)
            .map(|s| s.display_title().to_string())
            .unwrap_or_default();
        let index = self.tab_order.iter().position(|&t| t == id).unwrap_or(0);
        self.tab_bar_state.begin_close(id, title, index);

        if let Some(session) = self.sessions.remove(&id) {
            session.shutdown();
        }
        self.tab_order.retain(|&tab_id| tab_id != id);
        if self.active_tab == Some(id) {
            self.active_tab = self.tab_order.last().copied();
        }
    }

    /// Resize all sessions if the computed grid dimensions changed.
    pub fn resize_sessions(&mut self, cols: u16, rows: u16) {
        if cols == 0 || rows == 0 || (cols == self.last_cols && rows == self.last_rows) {
            return;
        }
        self.last_cols = cols;
        self.last_rows = rows;
        let cw = self.cell_width as u16;
        let ch = self.cell_height as u16;
        for session in self.sessions.values() {
            session.resize(cols, rows, cw, ch);
        }
    }

    /// Toggle zen mode: hide/show panels and status bar.
    pub fn toggle_zen_mode(&mut self) {
        if self.left_panel_visible || self.right_panel_visible || self.show_status_bar {
            self.left_panel_visible = false;
            self.right_panel_visible = false;
            self.show_status_bar = false;
        } else {
            self.left_panel_visible = true;
            self.right_panel_visible = true;
            self.show_status_bar = true;
        }
    }
}

// ── Tests ──

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_state() -> WindowState {
        WindowState::new(egui::ViewportId::from_hash_of("test"))
    }

    #[test]
    fn new_window_state_defaults() {
        let ws = make_test_state();
        assert!(ws.sessions.is_empty());
        assert!(ws.tab_order.is_empty());
        assert!(ws.active_tab.is_none());
        assert!(ws.left_panel_visible);
        assert!(ws.right_panel_visible);
        assert!(ws.bottom_panel_visible);
        assert!(ws.show_status_bar);
        assert!(!ws.should_close);
        assert!(!ws.has_focus);
    }

    #[test]
    fn active_session_returns_none_when_empty() {
        let ws = make_test_state();
        assert!(ws.active_session().is_none());
    }

    #[test]
    fn toggle_zen_mode_hides_panels_and_status_bar() {
        let mut ws = make_test_state();
        assert!(ws.left_panel_visible);
        assert!(ws.right_panel_visible);
        assert!(ws.show_status_bar);

        ws.toggle_zen_mode();
        assert!(!ws.left_panel_visible);
        assert!(!ws.right_panel_visible);
        assert!(!ws.show_status_bar);
    }

    #[test]
    fn toggle_zen_mode_restores_panels_and_status_bar() {
        let mut ws = make_test_state();
        ws.toggle_zen_mode(); // hide
        ws.toggle_zen_mode(); // restore

        assert!(ws.left_panel_visible);
        assert!(ws.right_panel_visible);
        assert!(ws.show_status_bar);
    }

    #[test]
    fn toggle_zen_mode_partial_visibility_hides_all() {
        let mut ws = make_test_state();
        ws.left_panel_visible = false;
        ws.right_panel_visible = false;
        ws.show_status_bar = true;

        ws.toggle_zen_mode();
        assert!(!ws.left_panel_visible);
        assert!(!ws.right_panel_visible);
        assert!(!ws.show_status_bar);
    }

    #[test]
    fn remove_session_from_empty_is_safe() {
        let mut ws = make_test_state();
        ws.remove_session(Uuid::new_v4());
        assert!(ws.sessions.is_empty());
    }

    #[test]
    fn resize_sessions_ignores_zero_dimensions() {
        let mut ws = make_test_state();
        ws.resize_sessions(0, 24);
        assert_eq!(ws.last_cols, 0);
        assert_eq!(ws.last_rows, 0);
    }

    #[test]
    fn resize_sessions_ignores_unchanged_dimensions() {
        let mut ws = make_test_state();
        ws.last_cols = 80;
        ws.last_rows = 24;
        ws.resize_sessions(80, 24);
        assert_eq!(ws.last_cols, 80);
        assert_eq!(ws.last_rows, 24);
    }

    #[test]
    fn shared_config_round_trip() {
        let user_config = config::UserConfig::default();
        let persistent = config::PersistentState::default();
        let scheme = conch_core::color_scheme::resolve_theme(&user_config.colors.theme);
        let colors = ResolvedColors::from_scheme(&scheme);
        let theme = UiTheme::from_colors(&colors, user_config.colors.appearance_mode);
        let shortcuts = ResolvedShortcuts::from_config(&user_config.conch.keyboard);

        let cfg = SharedConfig {
            user_config,
            persistent,
            colors,
            theme,
            theme_dirty: true,
            shortcuts,
            plugin_keybindings: Vec::new(),
            plugin_keybindings_version: 0,
        };

        assert!(cfg.theme_dirty);
        assert!(cfg.plugin_keybindings.is_empty());
    }
}
