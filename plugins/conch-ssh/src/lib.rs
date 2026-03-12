//! Conch SSH Plugin — real SSH connections via russh.

mod config;
mod server_tree;
mod session_backend;

use std::collections::HashMap;
use std::ffi::{CStr, CString};
use std::sync::Arc;

use conch_plugin_sdk::{
    widgets::{PluginEvent, Widget, WidgetEvent},
    HostApi, PanelHandle, PanelLocation, PluginInfo, PluginType,
    SessionHandle, SessionMeta,
};
use russh::client;
use tokio::runtime::Runtime;

use crate::config::{ServerEntry, SshConfig};
use crate::server_tree::build_server_tree;
use crate::session_backend::{SshBackendState, ssh_vtable};

/// The SSH plugin's runtime state.
struct SshPlugin {
    api: &'static HostApi,
    _panel: PanelHandle,
    config: SshConfig,
    /// Active SSH sessions keyed by host-assigned SessionHandle.
    sessions: HashMap<u64, Box<SshBackendState>>,
    selected_node: Option<String>,
    quick_connect_value: String,
    dirty: bool,
    /// Tokio runtime for async SSH operations.
    rt: Runtime,
}

// ---------------------------------------------------------------------------
// Plugin lifecycle
// ---------------------------------------------------------------------------

impl SshPlugin {
    fn new(api: &'static HostApi) -> Self {
        let msg = CString::new("SSH plugin initializing").unwrap();
        (api.log)(2, msg.as_ptr());

        let name = CString::new("Sessions").unwrap();
        let icon = CString::new("server.png").unwrap();
        let panel = (api.register_panel)(PanelLocation::Right, name.as_ptr(), icon.as_ptr());

        for svc in &["connect", "exec", "get_sessions", "get_handle"] {
            let svc_name = CString::new(*svc).unwrap();
            (api.register_service)(svc_name.as_ptr());
        }

        let tab_changed = CString::new("app.tab_changed").unwrap();
        (api.subscribe)(tab_changed.as_ptr());
        let theme_changed = CString::new("app.theme_changed").unwrap();
        (api.subscribe)(theme_changed.as_ptr());

        let menu = CString::new("File").unwrap();
        let label = CString::new("New SSH Connection...").unwrap();
        let action = CString::new("ssh.new_connection").unwrap();
        let keybind = CString::new("cmd+shift+s").unwrap();
        (api.register_menu_item)(
            menu.as_ptr(), label.as_ptr(),
            action.as_ptr(), keybind.as_ptr(),
        );

        let config = Self::load_config(api);

        let rt = Runtime::new().expect("failed to create tokio runtime");

        SshPlugin {
            api,
            _panel: panel,
            config,
            sessions: HashMap::new(),
            selected_node: None,
            quick_connect_value: String::new(),
            dirty: true,
            rt,
        }
    }

    fn load_config(api: &'static HostApi) -> SshConfig {
        let key = CString::new("servers").unwrap();
        let result = (api.get_config)(key.as_ptr());
        if result.is_null() {
            return SshConfig::default();
        }
        let json_str = unsafe { CStr::from_ptr(result) }.to_str().unwrap_or("{}");
        let config: SshConfig = serde_json::from_str(json_str).unwrap_or_default();
        (api.free_string)(result);
        config
    }

    fn save_config(&self) {
        let key = CString::new("servers").unwrap();
        let json = serde_json::to_string(&self.config).unwrap_or_default();
        let value = CString::new(json).unwrap();
        (self.api.set_config)(key.as_ptr(), value.as_ptr());
    }

    // -----------------------------------------------------------------------
    // Event handling
    // -----------------------------------------------------------------------

    fn handle_event(&mut self, event: PluginEvent) {
        match event {
            PluginEvent::Widget(widget_event) => self.handle_widget_event(widget_event),
            PluginEvent::MenuAction { action } => self.handle_menu_action(&action),
            PluginEvent::BusEvent { event_type, data } => {
                self.handle_bus_event(&event_type, data);
            }
            PluginEvent::BusQuery { .. } => {}
            PluginEvent::ThemeChanged { .. } => {
                self.dirty = true;
            }
            PluginEvent::Shutdown => {
                let handles: Vec<u64> = self.sessions.keys().copied().collect();
                for h in handles {
                    self.disconnect(SessionHandle(h));
                }
            }
        }
    }

    fn handle_widget_event(&mut self, event: WidgetEvent) {
        match event {
            WidgetEvent::ToolbarInputChanged { id, value } if id == "quick_connect" => {
                self.quick_connect_value = value;
            }
            WidgetEvent::ToolbarInputSubmit { id, value } if id == "quick_connect" => {
                self.quick_connect(&value);
            }
            WidgetEvent::TreeSelect { id: _, node_id } => {
                self.selected_node = Some(node_id);
                self.dirty = true;
            }
            WidgetEvent::TreeActivate { id: _, node_id } => {
                self.connect_to_server(&node_id);
            }
            WidgetEvent::TreeToggle { id: _, node_id, expanded } => {
                self.config.set_folder_expanded(&node_id, expanded);
                self.dirty = true;
            }
            WidgetEvent::TreeContextMenu { id: _, node_id, action } => {
                match action.as_str() {
                    "connect" => self.connect_to_server(&node_id),
                    "edit" => self.edit_server(&node_id),
                    "delete" => self.delete_server(&node_id),
                    "duplicate" => self.duplicate_server(&node_id),
                    "copy_host" => self.copy_host_to_clipboard(&node_id),
                    _ => {}
                }
            }
            WidgetEvent::ButtonClick { id } if id == "add_server" => {
                self.add_server_dialog(None);
            }
            WidgetEvent::ButtonClick { id } if id == "add_folder" => {
                self.add_folder_dialog();
            }
            _ => {}
        }
    }

    fn handle_menu_action(&mut self, action: &str) {
        match action {
            "ssh.new_connection" => self.add_server_dialog(None),
            _ => {}
        }
    }

    fn handle_bus_event(&mut self, event_type: &str, _data: serde_json::Value) {
        match event_type {
            "app.tab_changed" | "app.theme_changed" => {
                self.dirty = true;
            }
            _ => {}
        }
    }

    // -----------------------------------------------------------------------
    // Connection lifecycle
    // -----------------------------------------------------------------------

    fn connect_to_server(&mut self, node_id: &str) {
        let server = match self.config.find_server(node_id) {
            Some(s) => s.clone(),
            None => return,
        };

        // Password prompt if needed.
        let password = if server.auth_method == "password" {
            let msg = CString::new(format!("Password for {}@{}:", server.user, server.host)).unwrap();
            let default = CString::new("").unwrap();
            let result = (self.api.show_prompt)(msg.as_ptr(), default.as_ptr());
            if result.is_null() {
                return;
            }
            let pw = unsafe { CStr::from_ptr(result) }.to_str().unwrap_or("").to_string();
            (self.api.free_string)(result);
            Some(pw)
        } else {
            None
        };

        let connect_result = do_ssh_connect_sync(
            &server, password.as_deref(), self.api, &self.rt,
        );

        match connect_result {
            Ok((session_handle, backend_state)) => {
                self.sessions.insert(session_handle.0, backend_state);

                // Publish event.
                let event_type = CString::new("ssh.session_ready").unwrap();
                let event_data = serde_json::json!({
                    "session_id": session_handle.0,
                    "host": server.host,
                    "user": server.user,
                    "port": server.port,
                });
                let data_json = CString::new(event_data.to_string()).unwrap();
                let data_bytes = data_json.as_bytes();
                (self.api.publish_event)(event_type.as_ptr(), data_json.as_ptr(), data_bytes.len());

                // Toast notification.
                let notif = serde_json::json!({
                    "title": "Connected",
                    "body": format!("{}@{}", server.user, server.host),
                    "level": "info",
                    "duration_ms": 3000,
                });
                let notif_json = CString::new(notif.to_string()).unwrap();
                let notif_bytes = notif_json.as_bytes();
                (self.api.notify)(notif_json.as_ptr(), notif_bytes.len());

                self.dirty = true;
            }
            Err(e) => {
                let title = CString::new("Connection Failed").unwrap();
                let msg = CString::new(format!("{e}")).unwrap();
                (self.api.show_error)(title.as_ptr(), msg.as_ptr());
            }
        }
    }

    fn quick_connect(&mut self, input: &str) {
        let parts: Vec<&str> = input.splitn(2, '@').collect();
        let (user, host_port) = if parts.len() == 2 {
            (parts[0].to_string(), parts[1])
        } else {
            (std::env::var("USER").unwrap_or_else(|_| "root".to_string()), parts[0])
        };

        let parts: Vec<&str> = host_port.rsplitn(2, ':').collect();
        let (host, port) = if parts.len() == 2 {
            (parts[1].to_string(), parts[0].parse().unwrap_or(22))
        } else {
            (parts[0].to_string(), 22u16)
        };

        let entry = ServerEntry {
            id: uuid::Uuid::new_v4().to_string(),
            label: format!("{user}@{host}:{port}"),
            host,
            port,
            user,
            auth_method: "key".to_string(),
            key_path: None,
        };

        // Attempt key-based auth first, then fall back to password.
        let server_id = entry.id.clone();
        self.config.add_server(entry);
        self.connect_to_server(&server_id);
    }

    fn disconnect(&mut self, handle: SessionHandle) {
        if let Some(_backend) = self.sessions.remove(&handle.0) {
            (self.api.close_session)(handle);

            let event_type = CString::new("ssh.session_closed").unwrap();
            let data = serde_json::json!({ "session_id": handle.0 });
            let data_json = CString::new(data.to_string()).unwrap();
            let data_bytes = data_json.as_bytes();
            (self.api.publish_event)(event_type.as_ptr(), data_json.as_ptr(), data_bytes.len());
        }
        self.dirty = true;
    }

    // -----------------------------------------------------------------------
    // Server management dialogs
    // -----------------------------------------------------------------------

    fn add_server_dialog(&mut self, existing: Option<&ServerEntry>) {
        let form = serde_json::json!({
            "title": if existing.is_some() { "Edit Server" } else { "Add Server" },
            "fields": [
                { "id": "label", "type": "text", "label": "Name", "value": existing.map(|s| &s.label).unwrap_or(&String::new()) },
                { "id": "host", "type": "text", "label": "Host", "value": existing.map(|s| &s.host).unwrap_or(&String::new()) },
                { "id": "port", "type": "number", "label": "Port", "value": existing.map(|s| s.port).unwrap_or(22) },
                { "id": "user", "type": "text", "label": "Username", "value": existing.map(|s| &s.user).unwrap_or(&String::new()) },
                { "id": "auth_method", "type": "combo", "label": "Auth Method", "options": ["key", "password"], "value": existing.map(|s| s.auth_method.as_str()).unwrap_or("key") },
                { "id": "key_path", "type": "text", "label": "Key Path (optional)", "value": existing.and_then(|s| s.key_path.as_deref()).unwrap_or("") },
            ],
        });

        let json = CString::new(form.to_string()).unwrap();
        let json_bytes = json.as_bytes();
        let result = (self.api.show_form)(json.as_ptr(), json_bytes.len());
        if result.is_null() {
            return;
        }

        let result_str = unsafe { CStr::from_ptr(result) }.to_str().unwrap_or("{}");
        let form_data: serde_json::Value = serde_json::from_str(result_str).unwrap_or_default();
        (self.api.free_string)(result);

        let label = form_data["label"].as_str().unwrap_or("").to_string();
        let host = form_data["host"].as_str().unwrap_or("").to_string();
        let port = form_data["port"].as_f64().unwrap_or(22.0) as u16;
        let user = form_data["user"].as_str().unwrap_or("").to_string();
        let auth_method = form_data["auth_method"].as_str().unwrap_or("key").to_string();
        let key_path = form_data["key_path"].as_str()
            .filter(|s| !s.is_empty())
            .map(String::from);

        if host.is_empty() {
            return;
        }

        let id = existing
            .map(|e| e.id.clone())
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

        if existing.is_some() {
            self.config.remove_server(&id);
        }

        self.config.add_server(ServerEntry {
            id,
            label: if label.is_empty() { format!("{user}@{host}") } else { label },
            host,
            port,
            user,
            auth_method,
            key_path,
        });

        self.save_config();
        self.dirty = true;
    }

    fn add_folder_dialog(&mut self) {
        let msg = CString::new("Folder name:").unwrap();
        let default = CString::new("New Folder").unwrap();
        let result = (self.api.show_prompt)(msg.as_ptr(), default.as_ptr());
        if result.is_null() {
            return;
        }
        let name = unsafe { CStr::from_ptr(result) }.to_str().unwrap_or("").to_string();
        (self.api.free_string)(result);

        self.config.add_folder(&name);
        self.save_config();
        self.dirty = true;
    }

    fn edit_server(&mut self, node_id: &str) {
        let server = self.config.find_server(node_id).cloned();
        if let Some(s) = server.as_ref() {
            self.add_server_dialog(Some(s));
        }
    }

    fn delete_server(&mut self, node_id: &str) {
        let label = self.config.find_server(node_id)
            .map(|s| s.label.clone())
            .unwrap_or_default();

        let msg = CString::new(format!("Delete \"{label}\"?")).unwrap();
        let confirmed = (self.api.show_confirm)(msg.as_ptr());
        if confirmed {
            self.config.remove_server(node_id);
            self.save_config();
            self.dirty = true;
        }
    }

    fn duplicate_server(&mut self, node_id: &str) {
        if let Some(server) = self.config.find_server(node_id).cloned() {
            let mut dup = server;
            dup.id = uuid::Uuid::new_v4().to_string();
            dup.label = format!("{} (copy)", dup.label);
            self.config.add_server(dup);
            self.save_config();
            self.dirty = true;
        }
    }

    fn copy_host_to_clipboard(&self, node_id: &str) {
        if let Some(server) = self.config.find_server(node_id) {
            let text = CString::new(server.host.clone()).unwrap();
            (self.api.clipboard_set)(text.as_ptr());
        }
    }

    // -----------------------------------------------------------------------
    // Rendering
    // -----------------------------------------------------------------------

    fn render(&self) -> Vec<Widget> {
        build_server_tree(&self.config, &self.sessions, self.selected_node.as_deref())
    }

    // -----------------------------------------------------------------------
    // Service queries
    // -----------------------------------------------------------------------

    fn handle_query(&self, method: &str, args: serde_json::Value) -> serde_json::Value {
        match method {
            "get_sessions" => {
                let sessions: Vec<serde_json::Value> = self.sessions.iter().map(|(id, backend)| {
                    serde_json::json!({
                        "session_id": id,
                        "host": backend.host(),
                        "user": backend.user(),
                    })
                }).collect();
                serde_json::json!(sessions)
            }
            "exec" => {
                let session_id = args["session_id"].as_u64().unwrap_or(0);
                let command = args["command"].as_str().unwrap_or("");
                if self.sessions.contains_key(&session_id) {
                    // TODO: Open a separate SSH exec channel for this.
                    serde_json::json!({
                        "status": "ok",
                        "stdout": format!("(not yet implemented) executed: {command}"),
                        "exit_code": 0,
                    })
                } else {
                    serde_json::json!({ "status": "error", "message": "session not found" })
                }
            }
            "connect" => {
                let host = args["host"].as_str().unwrap_or("");
                serde_json::json!({ "status": "ok", "host": host })
            }
            "get_handle" => {
                let session_id = args["session_id"].as_u64().unwrap_or(0);
                if self.sessions.contains_key(&session_id) {
                    serde_json::json!({ "status": "ok", "session_id": session_id })
                } else {
                    serde_json::json!({ "status": "error", "message": "session not found" })
                }
            }
            _ => serde_json::json!({ "status": "error", "message": "unknown method" }),
        }
    }
}

// ---------------------------------------------------------------------------
// SSH connection logic
// ---------------------------------------------------------------------------

/// The russh client handler — implements host key verification via the host
/// dialog API.
struct SshHandler {
    api: &'static HostApi,
}

#[async_trait::async_trait]
impl client::Handler for SshHandler {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        _server_public_key: &ssh_key::PublicKey,
    ) -> Result<bool, Self::Error> {
        let fingerprint = _server_public_key.fingerprint(ssh_key::HashAlg::Sha256);
        let msg = CString::new(format!(
            "The server's SSH key fingerprint is:\n\n{fingerprint}\n\nDo you want to continue connecting?"
        )).unwrap();
        let accepted = (self.api.show_confirm)(msg.as_ptr());
        Ok(accepted)
    }
}

/// Perform the full SSH connection handshake and open a terminal channel.
///
/// Synchronous — blocks the plugin thread. The flow:
/// 1. SSH connect + authenticate (async, run on tokio runtime)
/// 2. Pre-allocate SshBackendState, pass pointer to host via open_session
/// 3. Host returns output callback
/// 4. Activate the backend state with the channel + callback
fn do_ssh_connect_sync(
    server: &ServerEntry,
    password: Option<&str>,
    api: &'static HostApi,
    rt: &Runtime,
) -> Result<(SessionHandle, Box<SshBackendState>), String> {
    // Phase 1: SSH handshake (async).
    let channel = rt.block_on(async {
        let config = Arc::new(client::Config::default());
        let handler = SshHandler { api };

        let addr = format!("{}:{}", server.host, server.port);
        let mut session = client::connect(config, &addr, handler)
            .await
            .map_err(|e| format!("Connection failed: {e}"))?;

        let authenticated = if server.auth_method == "password" {
            let pw = password.unwrap_or("");
            session.authenticate_password(&server.user, pw)
                .await
                .map_err(|e| format!("Auth failed: {e}"))?
        } else {
            try_key_auth(&mut session, &server.user, server.key_path.as_deref()).await?
        };

        if !authenticated {
            return Err("Authentication failed".to_string());
        }

        let channel = session.channel_open_session()
            .await
            .map_err(|e| format!("Channel open failed: {e}"))?;

        channel.request_pty(
            false, "xterm-256color", 80, 24, 0, 0,
            &[],
        ).await.map_err(|e| format!("PTY request failed: {e}"))?;

        channel.request_shell(false)
            .await
            .map_err(|e| format!("Shell request failed: {e}"))?;

        Ok::<_, String>(channel)
    })?;

    // Phase 2: Pre-allocate state and open session in the host.
    let mut backend_state = SshBackendState::new_preallocated(
        server.host.clone(),
        server.user.clone(),
    );

    let title = CString::new(format!("{}@{}", server.user, server.host)).unwrap();
    let short_title = CString::new(server.host.clone()).unwrap();
    let session_type = CString::new("ssh").unwrap();
    let meta = SessionMeta {
        title: title.as_ptr(),
        short_title: short_title.as_ptr(),
        session_type: session_type.as_ptr(),
        icon: std::ptr::null(),
    };

    let vtable = ssh_vtable();
    let backend_handle = SshBackendState::as_handle_ptr(&mut backend_state);
    let open_result = (api.open_session)(&meta, &vtable, backend_handle);
    let session_handle = open_result.handle;

    if session_handle.0 == 0 {
        return Err("Host refused to open session tab".to_string());
    }

    // Phase 3: Activate — wire up the channel and output callback.
    backend_state.activate(
        channel,
        open_result.output_cb,
        open_result.output_ctx,
        rt.handle(),
    );

    Ok((session_handle, backend_state))
}

/// Try key-based authentication with common SSH key files.
async fn try_key_auth(
    session: &mut client::Handle<SshHandler>,
    user: &str,
    explicit_key_path: Option<&str>,
) -> Result<bool, String> {
    let key_paths: Vec<std::path::PathBuf> = if let Some(path) = explicit_key_path {
        vec![std::path::PathBuf::from(path)]
    } else {
        let home = dirs::home_dir().unwrap_or_default();
        let ssh_dir = home.join(".ssh");
        vec![
            ssh_dir.join("id_ed25519"),
            ssh_dir.join("id_rsa"),
            ssh_dir.join("id_ecdsa"),
        ]
    };

    for key_path in &key_paths {
        if !key_path.exists() {
            continue;
        }

        match russh_keys::load_secret_key(key_path, None) {
            Ok(key) => {
                match session.authenticate_publickey(user, Arc::new(key)).await {
                    Ok(true) => return Ok(true),
                    Ok(false) => continue,
                    Err(_) => continue,
                }
            }
            Err(_) => continue,
        }
    }

    Ok(false)
}

// ---------------------------------------------------------------------------
// declare_plugin! macro
// ---------------------------------------------------------------------------

conch_plugin_sdk::declare_plugin!(
    info: PluginInfo {
        name: c"SSH Manager".as_ptr(),
        description: c"SSH connections and session management".as_ptr(),
        version: c"0.1.0".as_ptr(),
        plugin_type: PluginType::Panel,
        panel_location: PanelLocation::Right,
        dependencies: std::ptr::null(),
        num_dependencies: 0,
    },
    state: SshPlugin,
    setup: |api| SshPlugin::new(api),
    event: |state, event| state.handle_event(event),
    render: |state| state.render(),
    query: |state, method, args| state.handle_query(method, args),
);
