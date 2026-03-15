//! Main application struct and egui update loop.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use conch_core::config;
use conch_plugin::bus::PluginBus;
use conch_plugin::jvm::runtime::JavaPluginManager;
use conch_plugin::lua::runner::RunningLuaPlugin;
use conch_plugin::native::manager::NativePluginManager;
use egui::ViewportCommand;
use parking_lot::Mutex;
use tokio::sync::oneshot;

use crate::extra_window::ExtraWindow;
use crate::host::bridge::{self, PanelRegistry, SessionRegistry};
use crate::host::dialogs;
use crate::host::plugin_manager_ui::PluginManagerState;
use crate::input::ResolvedShortcuts;
use crate::ipc::{IpcListener, IpcMessage};
use crate::menu_bar::MenuBarState;
use crate::notifications::NotificationManager;
use crate::platform::PlatformCapabilities;
use crate::sessions::create_local_session;
use crate::terminal::color::ResolvedColors;
use crate::terminal::widget::{self, TerminalFrameCache};
use crate::watcher::{FileChangeKind, FileWatcher};
use crate::window_state::{SharedAppState, SharedConfig, WindowState};

/// Cursor blink interval in milliseconds.
const CURSOR_BLINK_MS: u128 = 500;

pub struct ConchApp {
    /// Shared global state (config, theme, plugin infra, etc.).
    pub(crate) shared: Arc<SharedAppState>,

    /// Primary window state (sessions, tabs, terminal rendering, UI chrome).
    pub(crate) main_window: WindowState,

    // Plugin managers.
    pub(crate) plugin_manager: PluginManagerState,
    pub(crate) native_plugin_mgr: NativePluginManager,
    /// Running Lua plugins, keyed by name.
    pub(crate) lua_plugins: HashMap<String, RunningLuaPlugin>,
    pub(crate) java_plugin_mgr: JavaPluginManager,
    /// Pending render responses from plugin threads (plugin_name → receiver).
    pub(crate) render_pending: HashMap<String, oneshot::Receiver<String>>,
    /// Last time a render request was sent to each plugin (for throttling).
    pub(crate) render_last_request: HashMap<String, Instant>,

    // Multi-window.
    pub(crate) extra_windows: Vec<ExtraWindow>,
    pub(crate) next_viewport_num: u32,

    // Tab change tracking (for plugin bus events).
    pub(crate) prev_active_tab: Option<uuid::Uuid>,

    // System.
    pub(crate) ipc_listener: Option<IpcListener>,
    pub(crate) file_watcher: Option<FileWatcher>,
    pub(crate) has_ever_had_session: bool,
    pub(crate) quit_requested: bool,
    pub(crate) rt: Arc<tokio::runtime::Runtime>,
}


impl ConchApp {
    pub fn new(rt: Arc<tokio::runtime::Runtime>) -> Self {
        let user_config = config::load_user_config().unwrap_or_else(|e| {
            log::error!("Failed to load config: {e:#}");
            config::UserConfig::default()
        });
        let persistent = config::load_persistent_state().unwrap_or_default();

        let shortcuts = ResolvedShortcuts::from_config(&user_config.conch.keyboard);
        let platform = PlatformCapabilities::current();
        let menu_bar_state = MenuBarState::new(user_config.conch.ui.native_menu_bar, &platform);

        let scheme = conch_core::color_scheme::resolve_theme(&user_config.colors.theme);
        let colors = ResolvedColors::from_scheme(&scheme);
        let theme = crate::ui_theme::UiTheme::from_colors(&colors, user_config.colors.appearance_mode);

        let ipc_listener = IpcListener::start();
        let file_watcher = FileWatcher::start();

        // Plugin infrastructure.
        let plugin_bus = Arc::new(PluginBus::new());
        let panel_registry = Arc::new(Mutex::new(PanelRegistry::new()));
        let (dialog_tx, dialog_state) = dialogs::dialog_channel();
        let session_registry = Arc::new(Mutex::new(SessionRegistry::new()));
        let notification_rx = crate::notifications::init_channel();
        let notifications = NotificationManager::new(notification_rx);
        bridge::init_bridge(
            Arc::clone(&plugin_bus),
            Arc::clone(&panel_registry),
            dialog_tx,
            Arc::clone(&session_registry),
        );
        let host_api = bridge::build_host_api();
        let java_host_api = bridge::build_host_api();
        let native_plugin_mgr = NativePluginManager::new(Arc::clone(&plugin_bus), host_api);
        let java_plugin_mgr = JavaPluginManager::new(Arc::clone(&plugin_bus), java_host_api);

        let shared_config = SharedConfig {
            user_config,
            persistent,
            colors,
            theme,
            theme_dirty: true,
            shortcuts,
            plugin_keybindings: Vec::new(),
            plugin_keybindings_version: 0,
        };

        let shared = Arc::new(SharedAppState {
            config: Mutex::new(shared_config),
            plugin_bus,
            panel_registry,
            session_registry,
            render_cache: Mutex::new(HashMap::new()),
            dialog_state: Mutex::new(dialog_state),
            notifications: Mutex::new(notifications),
            icon_cache: Mutex::new(None),
            menu_bar_state: Mutex::new(menu_bar_state),
            platform,
        });

        let main_window = WindowState::new(egui::ViewportId::ROOT);

        let mut app = Self {
            shared,
            main_window,
            plugin_manager: PluginManagerState::default(),
            native_plugin_mgr,
            lua_plugins: HashMap::new(),
            java_plugin_mgr,
            render_pending: HashMap::new(),
            render_last_request: HashMap::new(),
            extra_windows: Vec::new(),
            next_viewport_num: 1,
            prev_active_tab: None,
            ipc_listener,
            file_watcher,
            has_ever_had_session: false,
            quit_requested: false,
            rt,
        };

        // Discover plugins and auto-load previously enabled ones.
        app.discover_plugins();
        app.auto_load_plugins();

        app
    }

    /// Re-resolve plugin keybindings when menu items change.
    fn refresh_plugin_keybindings(&mut self) {
        use crate::input::{KeyBinding, ResolvedPluginKeybind};

        let version = bridge::plugin_menu_items_version();
        let mut cfg = self.shared.config.lock();
        if version == cfg.plugin_keybindings_version {
            return;
        }
        cfg.plugin_keybindings_version = version;

        cfg.plugin_keybindings = bridge::plugin_menu_items()
            .into_iter()
            .filter_map(|item| {
                let kb_str = item.keybind.as_deref()?;
                let binding = KeyBinding::parse(kb_str)?;
                Some(ResolvedPluginKeybind {
                    binding,
                    plugin_name: item.plugin_name,
                    action: item.action,
                })
            })
            .collect();
    }

    /// Build a `ViewportBuilder` for extra windows matching main window decorations.
    pub(crate) fn build_extra_viewport(&self) -> egui::ViewportBuilder {
        let cfg = self.shared.config.lock();
        let decorations = self.shared.platform.effective_decorations(
            cfg.user_config.window.decorations,
        );
        crate::build_viewport(
            egui::ViewportBuilder::default().with_inner_size([800.0, 600.0]),
            decorations,
            &self.shared.platform,
        )
    }

    /// Open a new OS window with a fresh local terminal tab.
    pub(crate) fn spawn_extra_window(&mut self) {
        let cwd = self.main_window.active_session()
            .and_then(|s| s.child_pid())
            .and_then(conch_pty::get_cwd_of_pid);
        let user_config = self.shared.config.lock().user_config.clone();
        let Some((_, session)) = create_local_session(&user_config, cwd) else {
            return;
        };
        let num = self.next_viewport_num;
        self.next_viewport_num += 1;
        let viewport_id = egui::ViewportId::from_hash_of(format!("conch_window_{num}"));
        let builder = self.build_extra_viewport();
        self.extra_windows.push(ExtraWindow::new(viewport_id, builder, session));
    }

    /// Poll terminal events for all main-window sessions.
    fn poll_events(&mut self, ctx: &egui::Context) {
        let mut exited_sessions = Vec::new();

        for (id, session) in &mut self.main_window.sessions {
            while let Ok(event) = session.event_rx.try_recv() {
                match event {
                    alacritty_terminal::event::Event::Wakeup => ctx.request_repaint(),
                    alacritty_terminal::event::Event::Title(title) => {
                        if session.custom_title.is_none() {
                            session.title = title;
                        }
                    }
                    alacritty_terminal::event::Event::Exit => {
                        exited_sessions.push(*id);
                    }
                    _ => {}
                }
            }
        }

        for id in exited_sessions {
            self.main_window.remove_session(id);
        }
    }

    /// Handle file watcher events.
    fn handle_file_changes(&mut self, ctx: &egui::Context) {
        let Some(watcher) = &mut self.file_watcher else { return };
        let changes = watcher.poll();
        for change in changes {
            match change.kind {
                FileChangeKind::Config => {
                    log::info!("Config file changed, reloading...");
                    match config::load_user_config() {
                        Ok(new_config) => {
                            let scheme = conch_core::color_scheme::resolve_theme(&new_config.colors.theme);
                            let colors = ResolvedColors::from_scheme(&scheme);
                            let theme = crate::ui_theme::UiTheme::from_colors(&colors, new_config.colors.appearance_mode);
                            let shortcuts = ResolvedShortcuts::from_config(&new_config.conch.keyboard);
                            crate::apply_appearance_mode(ctx, new_config.colors.appearance_mode);
                            self.shared.menu_bar_state.lock().update_mode(
                                new_config.conch.ui.native_menu_bar,
                                &self.shared.platform,
                            );
                            {
                                let mut cfg = self.shared.config.lock();
                                cfg.shortcuts = shortcuts;
                                cfg.colors = colors;
                                cfg.theme = theme;
                                cfg.theme_dirty = true;
                                cfg.user_config = new_config;
                            }
                            crate::notifications::push(crate::notifications::Notification::new(
                                Some("Config Reloaded".into()),
                                "Configuration updated successfully.".into(),
                                crate::notifications::NotificationLevel::Success,
                                None,
                            ));
                        }
                        Err(e) => {
                            crate::notifications::push(crate::notifications::Notification::new(
                                Some("Config Error".into()),
                                format!("Failed to reload config: {e}"),
                                crate::notifications::NotificationLevel::Error,
                                None,
                            ));
                        }
                    }
                }
                FileChangeKind::Themes => {
                    log::info!("Themes changed, reloading...");
                    let mut cfg = self.shared.config.lock();
                    let scheme = conch_core::color_scheme::resolve_theme(&cfg.user_config.colors.theme);
                    cfg.colors = ResolvedColors::from_scheme(&scheme);
                    cfg.theme = crate::ui_theme::UiTheme::from_colors(&cfg.colors, cfg.user_config.colors.appearance_mode);
                    cfg.theme_dirty = true;
                    drop(cfg);
                    crate::notifications::push(crate::notifications::Notification::new(
                        Some("Theme Reloaded".into()),
                        "Theme updated successfully.".into(),
                        crate::notifications::NotificationLevel::Success,
                        None,
                    ));
                }
            }
        }
    }

    /// Handle IPC messages from external processes.
    fn handle_ipc(&mut self) {
        let Some(listener) = &self.ipc_listener else { return };
        for msg in listener.drain() {
            match msg {
                IpcMessage::CreateWindow { working_directory } => {
                    let cwd = working_directory.map(std::path::PathBuf::from);
                    let user_config = self.shared.config.lock().user_config.clone();
                    if let Some((_, session)) = create_local_session(&user_config, cwd) {
                        let num = self.next_viewport_num;
                        self.next_viewport_num += 1;
                        let viewport_id = egui::ViewportId::from_hash_of(format!("conch_window_{num}"));
                        let builder = self.build_extra_viewport();
                        self.extra_windows.push(ExtraWindow::new(viewport_id, builder, session));
                    }
                }
                IpcMessage::CreateTab { working_directory } => {
                    let cwd = working_directory.map(std::path::PathBuf::from);
                    let user_config = self.shared.config.lock().user_config.clone();
                    if let Some((id, session)) = create_local_session(&user_config, cwd) {
                        self.main_window.sessions.insert(id, session);
                        self.main_window.tab_order.push(id);
                        self.main_window.active_tab = Some(id);
                    }
                }
            }
        }
    }

    /// Drain pending session open/close requests from plugins.
    fn drain_pending_sessions(&mut self) {
        let mut registry = self.shared.session_registry.lock();
        let pending: Vec<_> = registry.pending_open.drain(..).collect();
        let closing: Vec<_> = registry.pending_close.drain(..).collect();
        let status_updates: Vec<_> = registry.pending_status.drain(..).collect();
        drop(registry);

        // Process session opens.
        for mut ps in pending {
            let id = uuid::Uuid::new_v4();
            let event_rx = ps.bridge.take_event_rx();
            let session = crate::state::Session {
                id,
                title: ps.title,
                custom_title: None,
                backend: crate::state::SessionBackend::Plugin {
                    bridge: ps.bridge,
                    vtable: ps.vtable,
                    backend_handle: ps.backend_handle,
                },
                event_rx,
                status: conch_plugin_sdk::SessionStatus::Connecting,
                status_detail: None,
                connect_started: Some(std::time::Instant::now()),
                prompt: None,
            };

            // Route session to the window that triggered the interaction.
            let target = ps.target_viewport;
            let routed = target.and_then(|vp| {
                if vp == egui::ViewportId::ROOT {
                    return None; // main window — fall through
                }
                self.extra_windows.iter_mut().find(|w| w.viewport_id == vp)
            });

            if let Some(window) = routed {
                if window.last_cols > 0 && window.last_rows > 0 {
                    session.resize(
                        window.last_cols, window.last_rows,
                        window.cell_width as u16, window.cell_height as u16,
                    );
                }
                window.sessions.insert(id, session);
                window.tab_order.push(id);
                window.active_tab = Some(id);
            } else {
                if self.main_window.last_cols > 0 && self.main_window.last_rows > 0 {
                    session.resize(
                        self.main_window.last_cols, self.main_window.last_rows,
                        self.main_window.cell_width as u16, self.main_window.cell_height as u16,
                    );
                }
                self.main_window.sessions.insert(id, session);
                self.main_window.tab_order.push(id);
                self.main_window.active_tab = Some(id);
                self.has_ever_had_session = true;
            }
        }

        // Process session closes (search main window and extra windows).
        for handle in closing {
            // Try main window first.
            let main_id = self.main_window.sessions.iter().find_map(|(id, s)| {
                if let crate::state::SessionBackend::Plugin { bridge, .. } = &s.backend {
                    if bridge.handle == handle {
                        return Some(*id);
                    }
                }
                None
            });
            if let Some(id) = main_id {
                self.main_window.remove_session(id);
            } else {
                // Search extra windows.
                for window in &mut self.extra_windows {
                    let ew_id = window.sessions.iter().find_map(|(id, s)| {
                        if let crate::state::SessionBackend::Plugin { bridge, .. } = &s.backend {
                            if bridge.handle == handle {
                                return Some(*id);
                            }
                        }
                        None
                    });
                    if let Some(id) = ew_id {
                        if let Some(session) = window.sessions.remove(&id) {
                            session.shutdown();
                        }
                        window.tab_order.retain(|&tab_id| tab_id != id);
                        if window.active_tab == Some(id) {
                            window.active_tab = window.tab_order.last().copied();
                        }
                        break;
                    }
                }
            }
        }

        // Process session prompts (attach to the correct session).
        let prompts: Vec<_> = {
            let mut reg = self.shared.session_registry.lock();
            reg.pending_prompts.drain(..).collect()
        };
        for prompt_req in prompts {
            let handle = prompt_req.handle;
            let prompt_state = crate::state::SessionPrompt {
                prompt_type: prompt_req.prompt_type,
                message: prompt_req.message,
                detail: prompt_req.detail,
                password_buf: String::new(),
                focus_password: true,
                show_password: false,
                reply: Some(prompt_req.reply),
            };

            // Find the session in main window or extra windows.
            let main_match = self.main_window.sessions.values_mut().find(|s| {
                matches!(&s.backend, crate::state::SessionBackend::Plugin { bridge, .. } if bridge.handle == handle)
            });
            if let Some(session) = main_match {
                session.prompt = Some(prompt_state);
            } else {
                let mut placed = false;
                for window in &mut self.extra_windows {
                    let ew_match = window.sessions.values_mut().find(|s| {
                        matches!(&s.backend, crate::state::SessionBackend::Plugin { bridge, .. } if bridge.handle == handle)
                    });
                    if let Some(session) = ew_match {
                        session.prompt = Some(prompt_state);
                        placed = true;
                        break;
                    }
                }
                if !placed {
                    log::warn!("session_prompt: no session found for handle {:?}", handle);
                }
            }
        }

        // Process status updates (search main window and extra windows).
        for update in status_updates {
            let main_id = self.main_window.sessions.iter().find_map(|(id, s)| {
                if let crate::state::SessionBackend::Plugin { bridge, .. } = &s.backend {
                    if bridge.handle == update.handle {
                        return Some(*id);
                    }
                }
                None
            });
            if let Some(id) = main_id {
                if let Some(session) = self.main_window.sessions.get_mut(&id) {
                    session.status = update.status;
                    session.status_detail = update.detail;
                }
            } else {
                // Search extra windows.
                for window in &mut self.extra_windows {
                    let ew_id = window.sessions.iter().find_map(|(id, s)| {
                        if let crate::state::SessionBackend::Plugin { bridge, .. } = &s.backend {
                            if bridge.handle == update.handle {
                                return Some(*id);
                            }
                        }
                        None
                    });
                    if let Some(id) = ew_id {
                        if let Some(session) = window.sessions.get_mut(&id) {
                            session.status = update.status;
                            session.status_detail = update.detail;
                        }
                        break;
                    }
                }
            }
        }
    }

    /// Publish an `app.tab_changed` bus event so plugins can react to active
    /// session changes.
    fn publish_tab_changed(&self) {
        let (is_ssh, session_id) = if let Some(session) = self.main_window.active_session() {
            match &session.backend {
                crate::state::SessionBackend::Plugin { bridge, .. } => {
                    (true, Some(bridge.handle.0))
                }
                crate::state::SessionBackend::Local(_) => (false, None),
            }
        } else {
            (false, None)
        };

        let mut data = serde_json::json!({ "is_ssh": is_ssh });
        if let Some(sid) = session_id {
            data["session_id"] = serde_json::json!(sid);
        }

        self.shared.plugin_bus.publish("app", "app.tab_changed", data);
    }
}

impl eframe::App for ConchApp {
    /// Runs *before* egui's `begin_pass()` — strip Tab key events from raw
    /// input so egui never uses them for focus navigation. The Tab bytes are
    /// written directly to the active PTY session here.
    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        // If a dialog is open, let egui handle Tab normally.
        if self.shared.dialog_state.lock().is_active_for(egui::ViewportId::ROOT) {
            return;
        }

        let mut tab_bytes: Option<Vec<u8>> = None;
        raw_input.events.retain(|e| match e {
            egui::Event::Key {
                key: egui::Key::Tab,
                pressed: true,
                modifiers,
                ..
            } => {
                tab_bytes = Some(if modifiers.shift {
                    b"\x1b[Z".to_vec()
                } else {
                    b"\t".to_vec()
                });
                false
            }
            _ => true,
        });

        if let Some(bytes) = tab_bytes {
            if let Some(session) = self.main_window.active_session() {
                session.write(&bytes);
            }
        }
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Measure font cell size (and re-measure on DPI changes).
        let ppp = ctx.pixels_per_point();
        if !self.main_window.cell_size_measured || (ppp - self.main_window.last_pixels_per_point).abs() > 0.001 {
            let font_size = self.shared.config.lock().user_config.font.size;
            let (cw, ch) = widget::measure_cell_size(ctx, font_size);
            self.main_window.cell_width = cw;
            self.main_window.cell_height = ch;
            self.main_window.cell_size_measured = true;
            self.main_window.last_pixels_per_point = ppp;
        }

        // Cursor blink.
        let now = Instant::now();
        let elapsed = now.duration_since(self.main_window.last_blink).as_millis();
        if elapsed >= CURSOR_BLINK_MS {
            self.main_window.cursor_visible = !self.main_window.cursor_visible;
            self.main_window.last_blink = now;
            ctx.request_repaint_after(std::time::Duration::from_millis(CURSOR_BLINK_MS as u64));
        } else {
            let remaining = CURSOR_BLINK_MS - elapsed;
            ctx.request_repaint_after(std::time::Duration::from_millis(remaining as u64));
        }

        // Refresh plugin keybindings when menu items change.
        self.refresh_plugin_keybindings();

        // Detect tab changes and notify plugins via the bus.
        if self.main_window.active_tab != self.prev_active_tab {
            self.prev_active_tab = self.main_window.active_tab;
            self.publish_tab_changed();
        }

        // Poll events.
        self.poll_events(ctx);
        self.handle_file_changes(ctx);
        self.handle_ipc();
        self.poll_plugin_renders();
        self.drain_pending_sessions();

        // Show plugin dialogs (form, confirm, prompt, alert, error).
        self.shared.dialog_state.lock().show(ctx, egui::ViewportId::ROOT);

        // Render toast notifications on top of everything.
        self.shared.notifications.lock().show(ctx);

        // Determine whether the main window should hide or close.
        let mut main_visible = !self.main_window.sessions.is_empty();

        // If the user clicks close on the main window while extra windows exist,
        // hide it instead of quitting.
        let close_requested = ctx.input(|i| i.viewport().close_requested());
        if close_requested && !self.extra_windows.is_empty() {
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            // Shut down main-window sessions.
            let ids: Vec<_> = self.main_window.tab_order.clone();
            for id in ids {
                self.main_window.remove_session(id);
            }
            self.has_ever_had_session = true;
            main_visible = false;
        }

        // When the last main-window tab closes, either hide or close.
        if self.main_window.sessions.is_empty() {
            if !self.has_ever_had_session {
                let user_config = self.shared.config.lock().user_config.clone();
                self.main_window.open_local_tab(&user_config);
                self.has_ever_had_session = true;
                main_visible = true;
            } else {
                main_visible = false;
            }
        }

        // Show/hide the main viewport (extra windows are independent).
        if !main_visible {
            ctx.send_viewport_cmd(ViewportCommand::Visible(false));
        }

        // Main-window copy/paste handling.
        if main_visible {
            let copy_requested = ctx.input(|i| {
                i.events.iter().any(|e| matches!(e, egui::Event::Copy))
            });
            if copy_requested {
                if let Some((start, end)) = self.main_window.selection.normalized() {
                    if let Some(session) = self.main_window.active_session() {
                        let text = widget::get_selected_text(session.term(), start, end);
                        if !text.is_empty() {
                            ctx.copy_text(text);
                        }
                    }
                }
            }

            let paste_text: Option<String> = ctx.input(|i| {
                i.events.iter().find_map(|e| {
                    if let egui::Event::Paste(text) = e { Some(text.clone()) } else { None }
                })
            });
            if let Some(text) = paste_text {
                if let Some(session) = self.main_window.active_session() {
                    session.write(text.as_bytes());
                }
            }
        }

        // ── Render extra windows ──
        let effective_decorations = self.render_extra_windows(ctx);

        // If the main window is hidden and all extra windows are closed, quit.
        if !main_visible && self.extra_windows.is_empty() {
            ctx.send_viewport_cmd(ViewportCommand::Close);
            return;
        }

        // Skip main-window UI rendering when hidden.
        if !main_visible {
            return;
        }

        // ── Apply centralized UI theme (only when changed) ──
        {
            let mut cfg = self.shared.config.lock();
            if cfg.theme_dirty {
                cfg.theme.apply(ctx);
                bridge::update_theme_json(&cfg.theme);
                cfg.theme_dirty = false;
            }
        }
        let (bg_color, theme_clone) = {
            let cfg = self.shared.config.lock();
            (cfg.theme.bg, cfg.theme.clone())
        };

        // Buttonless: no native title bar, so add a thin drag region at the top.
        if effective_decorations == config::WindowDecorations::Buttonless {
            let drag_h = self.main_window.cell_height.max(6.0);
            egui::TopBottomPanel::top("drag_region")
                .exact_height(drag_h)
                .frame(egui::Frame::NONE.fill(theme_clone.bg_with_alpha(180)))
                .show(ctx, |ui| {
                    let rect = ui.available_rect_before_wrap();
                    let response = ui.interact(rect, ui.id().with("drag"), egui::Sense::drag());
                    if response.drag_started() {
                        ctx.send_viewport_cmd(ViewportCommand::StartDrag);
                    }
                });
        }

        // Full mode with fullsize_content_view (macOS): content extends behind
        // the native title bar, so add a spacer to push UI below it.
        if effective_decorations == config::WindowDecorations::Full
            && cfg!(target_os = "macos")
        {
            let title_bar_h = 34.0;
            egui::TopBottomPanel::top("titlebar_spacer")
                .exact_height(title_bar_h)
                .frame(egui::Frame::NONE.fill(theme_clone.surface))
                .show(ctx, |_ui| {});
        }

        // Tab bar at the top (only when more than one tab).
        {
            let tabs: Vec<(uuid::Uuid, String)> = self.main_window.tab_order.iter().map(|&id| {
                let title = self.main_window.sessions.get(&id)
                    .map(|s| s.display_title().to_string())
                    .unwrap_or_default();
                (id, title)
            }).collect();
            for action in crate::tab_bar::show_for(ctx, &tabs, self.main_window.active_tab, &theme_clone, &mut self.main_window.tab_bar_state) {
                match action {
                    crate::tab_bar::TabBarAction::SwitchTo(id) => {
                        self.main_window.active_tab = Some(id);
                    }
                    crate::tab_bar::TabBarAction::Close(id) => {
                        self.main_window.remove_session(id);
                    }
                }
            }
        }

        // Menu bar.
        let menu_action = crate::menu_bar::show(ctx, &mut *self.shared.menu_bar_state.lock());
        if let Some(action) = menu_action {
            // If an extra window has focus, route the action there instead.
            let focused_extra = self.extra_windows.iter_mut().find(|w| w.has_focus);
            if let Some(window) = focused_extra {
                window.pending_menu_actions.push(action);
            } else {
                crate::menu_bar::handle_action(action, ctx, self);
            }
        }

        // Plugin manager window (floating, toggled via View menu).
        if self.main_window.show_plugin_manager {
            let pm_actions = crate::host::plugin_manager_ui::show_plugin_manager_window(
                ctx,
                &mut self.main_window.show_plugin_manager,
                &mut self.plugin_manager,
                &theme_clone,
            );
            for pm_action in pm_actions {
                self.handle_plugin_manager_action(pm_action);
            }
        }

        // Lazy-init icon cache on first frame (needs egui context for textures).
        {
            let mut ic = self.shared.icon_cache.lock();
            if ic.is_none() {
                *ic = Some(crate::icons::IconCache::load(ctx));
            }
        }

        // Status bar at the very bottom edge.
        if self.main_window.show_status_bar {
            crate::host::plugin_panels::render_status_bar(ctx, &theme_clone);
        }

        // Render plugin panels (side panels, bottom panels).
        self.render_plugin_panels(ctx);

        // Central panel: terminal.
        let mut pending_resize: Option<(u16, u16)> = None;
        let mut context_action: Option<crate::menu_bar::MenuAction> = None;

        let mut close_tab_requested = false;
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(bg_color))
            .show(ctx, |ui| {
                if let Some(session) = self.main_window.active_tab.and_then(|id| self.main_window.sessions.get_mut(&id)) {
                    match session.status {
                        conch_plugin_sdk::SessionStatus::Connecting => {
                            let icon_cache = self.shared.icon_cache.lock();
                            let action = show_connecting_screen(
                                ui,
                                &session.title,
                                session.status_detail.as_deref(),
                                session.connect_started,
                                session.prompt.as_mut(),
                                icon_cache.as_ref(),
                            );
                            drop(icon_cache);
                            match action {
                                ConnectingAction::Accept => {
                                    if let Some(prompt) = session.prompt.take() {
                                        if let Some(reply) = prompt.reply {
                                            let _ = reply.send(Some("true".to_string()));
                                        }
                                    }
                                }
                                ConnectingAction::Reject => {
                                    if let Some(prompt) = session.prompt.take() {
                                        if let Some(reply) = prompt.reply {
                                            let _ = reply.send(None);
                                        }
                                    }
                                }
                                ConnectingAction::SubmitPassword(pw) => {
                                    if let Some(prompt) = session.prompt.take() {
                                        if let Some(reply) = prompt.reply {
                                            let _ = reply.send(Some(pw));
                                        }
                                    }
                                }
                                ConnectingAction::None => {}
                            }
                        }
                        conch_plugin_sdk::SessionStatus::Error => {
                            let detail = session.status_detail.clone().unwrap_or_default();
                            if show_error_screen(ui, &session.title, &detail) {
                                close_tab_requested = true;
                            }
                        }
                        conch_plugin_sdk::SessionStatus::Connected => {
                            let sel = self.main_window.selection.normalized();
                            let term = session.term();
                            let cfg = self.shared.config.lock();
                            let (response, size_info) = widget::show_terminal(
                                ui,
                                term,
                                self.main_window.cell_width,
                                self.main_window.cell_height,
                                &cfg.colors,
                                cfg.user_config.font.size,
                                self.main_window.cursor_visible,
                                sel,
                                &mut self.main_window.frame_cache,
                            );
                            drop(cfg);

                            pending_resize = Some((size_info.columns() as u16, size_info.rows() as u16));

                            // Check mouse mode for context menu suppression.
                            let mouse_mode = term
                                .try_lock_unfair()
                                .map(|t| t.mode().intersects(alacritty_terminal::term::TermMode::MOUSE_MODE))
                                .unwrap_or(false);

                            // Mouse handling.
                            let scroll_sensitivity = self.shared.config.lock().user_config.terminal.scroll_sensitivity;
                            crate::mouse::handle_terminal_mouse(
                                ctx,
                                &response,
                                &size_info,
                                &mut self.main_window.selection,
                                term,
                                &|bytes| session.write(bytes),
                                self.main_window.cell_height,
                                scroll_sensitivity,
                            );

                            // Context menu.
                            let has_selection = self.main_window.selection.normalized().is_some();
                            context_action = crate::context_menu::show(
                                &response,
                                &mut self.main_window.context_menu_state,
                                mouse_mode,
                                has_selection,
                            );
                        }
                    }
                }
            });

        // Handle close-tab request from error screen.
        if close_tab_requested {
            if let Some(id) = self.main_window.active_tab {
                self.main_window.remove_session(id);
            }
        }

        // Handle context menu action outside the panel closure.
        if let Some(action) = context_action {
            crate::menu_bar::handle_action(action, ctx, self);
        }

        // Resize sessions after releasing the panel borrow.
        if let Some((cols, rows)) = pending_resize {
            self.main_window.resize_sessions(cols, rows);
        }

        // Keyboard handling — forward to PTY unless a dialog or text input is consuming input.
        let text_edit_focused = ctx.memory(|m| m.focused()).is_some()
            && ctx.wants_keyboard_input();
        let forward_to_pty = !self.shared.dialog_state.lock().is_active_for(egui::ViewportId::ROOT)
            && !text_edit_focused;
        self.handle_keyboard(ctx, forward_to_pty);

        // Quit handling.
        if self.quit_requested {
            ctx.send_viewport_cmd(ViewportCommand::Close);
        }

        // Update window title from active session.
        if let Some(session) = self.main_window.active_session() {
            let title = format!("{} — Conch", session.display_title());
            ctx.send_viewport_cmd(ViewportCommand::Title(title));
        }

        // Save window size on each frame (debounced by OS).
        let rect = ctx.input(|i| i.screen_rect());
        if rect.width() > 100.0 && rect.height() > 100.0 {
            let mut cfg = self.shared.config.lock();
            cfg.persistent.layout.window_width = rect.width();
            cfg.persistent.layout.window_height = rect.height();
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.save_loaded_plugins();
        // Shut down Lua plugins.
        let lua_names: Vec<String> = self.lua_plugins.keys().cloned().collect();
        for name in lua_names {
            if let Some(mut running) = self.lua_plugins.remove(&name) {
                let _ = running.sender.try_send(conch_plugin::bus::PluginMail::Shutdown);
                if let Some(handle) = running.thread.take() {
                    let _ = handle.join();
                }
                self.shared.plugin_bus.unregister_plugin(&name);
            }
        }
        self.native_plugin_mgr.shutdown_all();
        self.java_plugin_mgr.shutdown_all();
        let cfg = self.shared.config.lock();
        let _ = config::save_persistent_state(&cfg.persistent);
    }
}

// ---------------------------------------------------------------------------
// Connecting / Error screens for plugin sessions
// ---------------------------------------------------------------------------

/// Action from the connecting screen (prompt response or close).
pub(crate) enum ConnectingAction {
    None,
    Accept,
    Reject,
    SubmitPassword(String),
}

/// Render a "Connecting to..." screen with optional inline prompts.
pub(crate) fn show_connecting_screen(
    ui: &mut egui::Ui,
    title: &str,
    detail: Option<&str>,
    started: Option<std::time::Instant>,
    prompt: Option<&mut crate::state::SessionPrompt>,
    icon_cache: Option<&crate::icons::IconCache>,
) -> ConnectingAction {
    let rect = ui.available_rect_before_wrap();
    let bg = if ui.visuals().dark_mode {
        egui::Color32::from_gray(30)
    } else {
        egui::Color32::from_gray(241)
    };
    ui.painter().rect_filled(rect, 0.0, bg);
    let center = rect.center();

    if let Some(prompt) = prompt {
        let content_width = (rect.width() * 0.7).min(560.0);
        let content_rect = egui::Rect::from_center_size(
            center,
            egui::Vec2::new(content_width, rect.height() * 0.7),
        );
        let mut action = ConnectingAction::None;

        if prompt.prompt_type == 0 {
            ui.allocate_new_ui(egui::UiBuilder::new().max_rect(content_rect), |ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(20.0);
                    let is_changed = prompt.message.contains("HAS CHANGED");
                    if is_changed {
                        ui.label(
                            egui::RichText::new("WARNING: HOST KEY HAS CHANGED!")
                                .size(22.0).strong()
                                .color(egui::Color32::from_rgb(220, 50, 50)),
                        );
                        ui.add_space(8.0);
                        ui.label(egui::RichText::new(&prompt.message).size(13.0)
                            .color(if ui.visuals().dark_mode { egui::Color32::from_gray(180) } else { egui::Color32::from_gray(60) }));
                    } else {
                        ui.label(egui::RichText::new(&prompt.message).size(18.0));
                    }
                    if !prompt.detail.is_empty() {
                        ui.add_space(16.0);
                        ui.label(egui::RichText::new(&prompt.detail).size(15.0)
                            .family(egui::FontFamily::Monospace).strong());
                    }
                    ui.add_space(20.0);
                    ui.label(egui::RichText::new("Are you sure you want to continue connecting?").size(14.0));
                    ui.add_space(12.0);
                    let btn_size = egui::Vec2::new(120.0, 34.0);
                    ui.horizontal(|ui| {
                        let total_w = btn_size.x * 2.0 + ui.spacing().item_spacing.x;
                        let avail = ui.available_width();
                        if avail > total_w { ui.add_space((avail - total_w) / 2.0); }
                        if ui.add_sized(btn_size, egui::Button::new("Accept")).clicked() {
                            action = ConnectingAction::Accept;
                        }
                        if ui.add_sized(btn_size, egui::Button::new("Reject")).clicked() {
                            action = ConnectingAction::Reject;
                        }
                    });
                });
            });
        } else {
            ui.allocate_new_ui(egui::UiBuilder::new().max_rect(content_rect), |ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(20.0);
                    ui.label(egui::RichText::new(&prompt.message).size(22.0));
                    if !prompt.detail.is_empty() {
                        ui.add_space(4.0);
                        ui.label(egui::RichText::new(&prompt.detail).size(14.0)
                            .color(if ui.visuals().dark_mode { egui::Color32::from_gray(160) } else { egui::Color32::from_gray(80) }));
                    }
                    ui.add_space(16.0);
                    let field_width = 340.0;
                    let field_height = 34.0;
                    let btn_zone = 32.0;
                    let (outer_rect, _) = ui.allocate_exact_size(
                        egui::Vec2::new(field_width, field_height), egui::Sense::hover());
                    let visuals = ui.visuals();
                    let rounding = egui::CornerRadius::same(6);
                    let stroke = visuals.widgets.active.bg_stroke;
                    ui.painter().rect(outer_rect, rounding, visuals.widgets.inactive.bg_fill, stroke, egui::StrokeKind::Outside);
                    let text_rect = egui::Rect::from_min_max(
                        outer_rect.min, egui::Pos2::new(outer_rect.max.x - btn_zone, outer_rect.max.y));
                    let mut text_child = ui.new_child(
                        egui::UiBuilder::new().max_rect(text_rect.shrink2(egui::vec2(8.0, 0.0))));
                    let pw_resp = text_child.add(
                        egui::TextEdit::singleline(&mut prompt.password_buf)
                            .password(!prompt.show_password).frame(false)
                            .margin(egui::Margin { left: 0, right: 0, top: 8, bottom: 4 })
                            .font(egui::TextStyle::Body)
                            .desired_width(text_rect.width() - 16.0)
                            .hint_text("Password"));
                    if prompt.focus_password { pw_resp.request_focus(); prompt.focus_password = false; }
                    let enter_pressed = pw_resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    let btn_rect = egui::Rect::from_min_max(
                        egui::Pos2::new(outer_rect.max.x - btn_zone, outer_rect.min.y), outer_rect.max).shrink(4.0);
                    let dark_mode = ui.visuals().dark_mode;
                    let can_submit = !prompt.password_buf.is_empty();
                    let tooltip = if prompt.show_password { "Hide password" } else { "Show password" };
                    let icon_size = egui::vec2(16.0, 16.0);
                    let icon_pos = egui::Pos2::new(btn_rect.center().x - 8.0, btn_rect.center().y - 8.0);
                    let icon_rect = egui::Rect::from_min_size(icon_pos, icon_size);
                    let eye_resp = ui.allocate_rect(icon_rect, egui::Sense::click());
                    if let Some(img) = icon_cache
                        .and_then(|ic| ic.themed_image(crate::icons::Icon::Eye, dark_mode))
                    {
                        img.fit_to_exact_size(icon_size).paint_at(ui, icon_rect);
                    }
                    if eye_resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(tooltip).clicked() {
                        prompt.show_password = !prompt.show_password;
                    }
                    if enter_pressed && can_submit {
                        action = ConnectingAction::SubmitPassword(prompt.password_buf.clone());
                    }
                    ui.add_space(8.0);
                    let cancel_text = egui::RichText::new("Cancel").size(13.0)
                        .color(if ui.visuals().dark_mode { egui::Color32::from_gray(140) } else { egui::Color32::from_gray(100) });
                    if ui.add(egui::Label::new(cancel_text).sense(egui::Sense::click())).clicked() {
                        action = ConnectingAction::Reject;
                    }
                });
            });
        }
        return action;
    }

    // Normal connecting screen (no prompt).
    let heading = format!("Connecting to {title}\u{2026}");
    let heading_galley = ui.painter().layout_no_wrap(
        heading, egui::FontId::new(28.0, egui::FontFamily::Proportional),
        if ui.visuals().dark_mode { egui::Color32::WHITE } else { egui::Color32::BLACK });
    let heading_pos = egui::Pos2::new(center.x - heading_galley.size().x / 2.0, center.y - 40.0);
    ui.painter().galley(heading_pos, heading_galley, egui::Color32::PLACEHOLDER);

    if let Some(detail) = detail {
        let detail_galley = ui.painter().layout_no_wrap(
            detail.to_string(), egui::FontId::new(16.0, egui::FontFamily::Proportional),
            if ui.visuals().dark_mode { egui::Color32::from_gray(200) } else { egui::Color32::from_gray(40) });
        let detail_pos = egui::Pos2::new(center.x - detail_galley.size().x / 2.0, center.y + 5.0);
        ui.painter().galley(detail_pos, detail_galley, egui::Color32::PLACEHOLDER);
    }

    let bar_w = 400.0_f32.min(rect.width() * 0.6);
    let bar_h = 6.0;
    let bar_y = center.y + 50.0;
    let bar_rect = egui::Rect::from_min_size(
        egui::Pos2::new(center.x - bar_w / 2.0, bar_y), egui::Vec2::new(bar_w, bar_h));
    let track_color = if ui.visuals().dark_mode { egui::Color32::from_gray(60) } else { egui::Color32::from_gray(210) };
    ui.painter().rect_filled(bar_rect, bar_h / 2.0, track_color);

    let elapsed = started.map(|s| s.elapsed().as_secs_f32()).unwrap_or(0.0);
    let cycle = 1.8;
    let t = (elapsed % cycle) / cycle;
    let pos_t = if t < 0.5 { t * 2.0 } else { 2.0 - t * 2.0 };
    let eased = pos_t * pos_t * (3.0 - 2.0 * pos_t);
    let indicator_w = bar_w * 0.15;
    let indicator_x = bar_rect.min.x + eased * (bar_w - indicator_w);
    let indicator_rect = egui::Rect::from_min_size(
        egui::Pos2::new(indicator_x, bar_y), egui::Vec2::new(indicator_w, bar_h));
    ui.painter().rect_filled(indicator_rect, bar_h / 2.0, egui::Color32::from_rgb(66, 133, 244));

    ConnectingAction::None
}

/// Render a connection error screen. Returns `true` if the user clicked "Close Tab".
pub(crate) fn show_error_screen(ui: &mut egui::Ui, title: &str, error: &str) -> bool {
    let rect = ui.available_rect_before_wrap();
    let bg = if ui.visuals().dark_mode { egui::Color32::from_gray(30) } else { egui::Color32::from_gray(241) };
    ui.painter().rect_filled(rect, 0.0, bg);
    let center = rect.center();
    let content_width = (rect.width() * 0.7).min(600.0);
    let content_rect = egui::Rect::from_center_size(center, egui::Vec2::new(content_width, rect.height() * 0.8));
    let mut close = false;
    ui.allocate_new_ui(egui::UiBuilder::new().max_rect(content_rect), |ui| {
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(20.0);
                ui.label(egui::RichText::new(format!("Connection to {title} failed"))
                    .size(24.0).color(egui::Color32::from_rgb(220, 50, 50)));
                ui.add_space(16.0);
            });
            let error_color = if ui.visuals().dark_mode { egui::Color32::from_gray(180) } else { egui::Color32::from_gray(60) };
            ui.label(egui::RichText::new(error).size(13.0).family(egui::FontFamily::Monospace).color(error_color));
            ui.add_space(16.0);
            ui.vertical_centered(|ui| { if ui.button("Close Tab").clicked() { close = true; } });
            ui.add_space(12.0);
        });
    });
    close
}
