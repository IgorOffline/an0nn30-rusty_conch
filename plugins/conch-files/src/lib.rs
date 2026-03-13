//! Conch Files Plugin — dual-pane local & remote file explorer with transfer.

mod format;
pub(crate) mod local;
pub(crate) mod pane;
mod remote;

use std::collections::HashMap;
use std::ffi::CString;

use conch_plugin_sdk::{
    declare_plugin,
    widgets::{PluginEvent, SplitDirection, TextStyle, Widget, WidgetEvent},
    HostApi, PanelHandle, PanelLocation, PluginInfo, PluginType,
};

use pane::{Pane, PaneMode};

/// Log a message through the HostApi.
fn host_log(api: &HostApi, level: u8, msg: &str) {
    if let Ok(c) = CString::new(msg) {
        (api.log)(level, c.as_ptr());
    }
}

/// A single file/directory entry.
pub struct FileEntry {
    pub name: String,
    pub is_dir: bool,
    pub size: u64,
    pub modified: Option<u64>,
}

/// Cached info about an SSH session.
struct SshSessionInfo {
    host: String,
    user: String,
}

/// The dual-pane file explorer plugin state.
struct FilesPlugin {
    api: &'static HostApi,
    _panel: PanelHandle,

    /// Top pane — always local.
    local_pane: Pane,
    /// Bottom pane — remote when SSH active, local otherwise.
    remote_pane: Pane,

    /// Known SSH sessions.
    ssh_sessions: HashMap<u64, SshSessionInfo>,
    active_session_id: Option<u64>,

    /// Transfer status message (shown between the panes).
    transfer_status: Option<(String, TextStyle)>,
}

impl FilesPlugin {
    fn new(api: &'static HostApi) -> Self {
        host_log(api, 2, "Files plugin initializing");

        let name = CString::new("Files").unwrap();
        let icon = CString::new("tab-files").unwrap();
        let panel = (api.register_panel)(PanelLocation::Left, name.as_ptr(), icon.as_ptr());

        for event in &["ssh.session_ready", "ssh.session_closed", "app.tab_changed"] {
            let ev = CString::new(*event).unwrap();
            (api.subscribe)(ev.as_ptr());
        }

        FilesPlugin {
            api,
            _panel: panel,
            local_pane: Pane::new_local("local"),
            remote_pane: Pane::new_local("remote"),
            ssh_sessions: HashMap::new(),
            active_session_id: None,
            transfer_status: None,
        }
    }

    // -------------------------------------------------------------------
    // Event handling
    // -------------------------------------------------------------------

    fn handle_event(&mut self, event: PluginEvent) {
        match event {
            PluginEvent::Widget(widget_event) => self.handle_widget_event(widget_event),
            PluginEvent::BusEvent { event_type, data } => {
                self.handle_bus_event(&event_type, data);
            }
            PluginEvent::Shutdown => {
                host_log(self.api, 2, "Files plugin shutting down");
            }
            _ => {}
        }
    }

    fn handle_widget_event(&mut self, event: WidgetEvent) {
        // Transfer buttons.
        if let WidgetEvent::ButtonClick { ref id } = event {
            match id.as_str() {
                "transfer_download" => {
                    self.do_download();
                    return;
                }
                "transfer_upload" => {
                    self.do_upload();
                    return;
                }
                _ => {}
            }
        }

        // Route to the correct pane.
        let api = Some(self.api as &HostApi);
        if !self.local_pane.handle_widget_event(&event, api) {
            self.remote_pane.handle_widget_event(&event, api);
        }
    }

    fn handle_bus_event(&mut self, event_type: &str, data: serde_json::Value) {
        match event_type {
            "ssh.session_ready" => {
                let session_id = data["session_id"].as_u64().unwrap_or(0);
                let host = data["host"].as_str().unwrap_or("").to_string();
                let user = data["user"].as_str().unwrap_or("").to_string();

                host_log(
                    self.api,
                    1,
                    &format!("SSH session ready: {session_id} ({user}@{host})"),
                );

                self.ssh_sessions.insert(session_id, SshSessionInfo {
                    host: host.clone(),
                    user: user.clone(),
                });

                // Switch the remote pane to the new SSH session.
                self.active_session_id = Some(session_id);
                self.remote_pane.switch_to_remote(session_id, &host, &user, self.api);
            }

            "ssh.session_closed" => {
                let session_id = data["session_id"].as_u64().unwrap_or(0);
                self.ssh_sessions.remove(&session_id);

                // If the closed session was our active remote, fall back to local.
                if self.active_session_id == Some(session_id) {
                    self.active_session_id = None;
                    self.remote_pane.switch_to_local();
                }
            }

            "app.tab_changed" => {
                let session_id = data["session_id"].as_u64();
                let is_ssh = data["is_ssh"].as_bool().unwrap_or(false);

                if is_ssh {
                    if let Some(sid) = session_id {
                        if let Some(info) = self.ssh_sessions.get(&sid) {
                            let host = info.host.clone();
                            let user = info.user.clone();
                            self.active_session_id = Some(sid);
                            // Only switch if not already on this session.
                            let already = matches!(
                                &self.remote_pane.mode,
                                PaneMode::Remote { session_id: active, .. } if *active == sid
                            );
                            if !already {
                                self.remote_pane.switch_to_remote(sid, &host, &user, self.api);
                            }
                        }
                    }
                } else {
                    self.active_session_id = None;
                    self.remote_pane.switch_to_local();
                }
            }

            _ => {}
        }
    }

    // -------------------------------------------------------------------
    // Transfer operations
    // -------------------------------------------------------------------

    /// Download: remote pane selection → local pane directory.
    fn do_download(&mut self) {
        let Some(remote_file) = self.remote_pane.selected_row.clone() else {
            self.transfer_status = Some(("No file selected in remote pane".into(), TextStyle::Warn));
            return;
        };

        if self.remote_pane.selected_is_dir() {
            self.transfer_status = Some(("Cannot transfer directories (yet)".into(), TextStyle::Warn));
            return;
        }

        let remote_path = self.remote_pane.selected_path().unwrap();
        let local_dest = if self.local_pane.current_path.ends_with('/') {
            format!("{}{}", self.local_pane.current_path, remote_file)
        } else {
            format!("{}/{}", self.local_pane.current_path, remote_file)
        };

        let result = match &self.remote_pane.mode {
            PaneMode::Remote { session_id, .. } => {
                // SFTP download: read from remote, write locally.
                match remote::read_file(self.api, *session_id, &remote_path) {
                    Ok(data) => std::fs::write(&local_dest, &data).map_err(|e| e.to_string()),
                    Err(e) => Err(e),
                }
            }
            PaneMode::Local => {
                // Both local: copy file.
                std::fs::copy(&remote_path, &local_dest)
                    .map(|_| ())
                    .map_err(|e| e.to_string())
            }
        };

        match result {
            Ok(()) => {
                self.transfer_status = Some((
                    format!("Downloaded: {remote_file}"),
                    TextStyle::Secondary,
                ));
                self.local_pane.refresh(Some(self.api));
            }
            Err(e) => {
                self.transfer_status = Some((
                    format!("Download failed: {e}"),
                    TextStyle::Error,
                ));
            }
        }
    }

    /// Upload: local pane selection → remote pane directory.
    fn do_upload(&mut self) {
        let Some(local_file) = self.local_pane.selected_row.clone() else {
            self.transfer_status = Some(("No file selected in local pane".into(), TextStyle::Warn));
            return;
        };

        if self.local_pane.selected_is_dir() {
            self.transfer_status = Some(("Cannot transfer directories (yet)".into(), TextStyle::Warn));
            return;
        }

        let local_path = self.local_pane.selected_path().unwrap();
        let remote_dest = if self.remote_pane.current_path.ends_with('/') || self.remote_pane.current_path == "." {
            if self.remote_pane.current_path == "." {
                local_file.clone()
            } else {
                format!("{}{}", self.remote_pane.current_path, local_file)
            }
        } else {
            format!("{}/{}", self.remote_pane.current_path, local_file)
        };

        let result = match &self.remote_pane.mode {
            PaneMode::Remote { session_id, .. } => {
                // SFTP upload: read locally, write to remote.
                match std::fs::read(&local_path) {
                    Ok(data) => remote::write_file(self.api, *session_id, &remote_dest, &data),
                    Err(e) => Err(e.to_string()),
                }
            }
            PaneMode::Local => {
                // Both local: copy file.
                std::fs::copy(&local_path, &remote_dest)
                    .map(|_| ())
                    .map_err(|e| e.to_string())
            }
        };

        match result {
            Ok(()) => {
                self.transfer_status = Some((
                    format!("Uploaded: {local_file}"),
                    TextStyle::Secondary,
                ));
                self.remote_pane.refresh(Some(self.api));
            }
            Err(e) => {
                self.transfer_status = Some((
                    format!("Upload failed: {e}"),
                    TextStyle::Error,
                ));
            }
        }
    }

    // -------------------------------------------------------------------
    // Rendering
    // -------------------------------------------------------------------

    fn render(&self) -> Vec<Widget> {
        let local_widgets = self.local_pane.render_widgets();
        let remote_widgets = self.remote_pane.render_widgets();

        // Top half: local pane.
        let local_container = Widget::Vertical {
            id: Some("local_container".into()),
            children: local_widgets,
            spacing: Some(2.0),
        };

        // Bottom half: transfer bar + remote pane.
        let mut transfer_children: Vec<Widget> = Vec::new();

        transfer_children.push(Widget::Button {
            id: "transfer_upload".into(),
            label: "\u{2191} Upload".into(),
            icon: None,
            enabled: Some(self.local_pane.selected_row.is_some() && !self.local_pane.selected_is_dir()),
        });

        transfer_children.push(Widget::Button {
            id: "transfer_download".into(),
            label: "\u{2193} Download".into(),
            icon: None,
            enabled: Some(self.remote_pane.selected_row.is_some() && !self.remote_pane.selected_is_dir()),
        });

        if let Some((msg, style)) = &self.transfer_status {
            transfer_children.push(Widget::Label {
                text: msg.clone(),
                style: Some(style.clone()),
            });
        }

        let transfer_bar = Widget::Horizontal {
            id: Some("transfer_bar".into()),
            children: transfer_children,
            spacing: Some(8.0),
        };

        let remote_container = Widget::Vertical {
            id: Some("remote_container".into()),
            children: remote_widgets,
            spacing: Some(2.0),
        };

        // Bottom half includes transfer bar above the remote pane.
        let bottom = Widget::Vertical {
            id: Some("bottom_half".into()),
            children: vec![transfer_bar, remote_container],
            spacing: Some(4.0),
        };

        vec![
            Widget::SplitPane {
                id: "file_split".into(),
                direction: SplitDirection::Vertical,
                ratio: 0.5,
                resizable: false,
                left: Box::new(local_container),
                right: Box::new(bottom),
            },
        ]
    }

    fn handle_query(&mut self, _method: &str, _args: serde_json::Value) -> serde_json::Value {
        serde_json::json!({ "status": "error", "message": "not implemented" })
    }
}

declare_plugin!(
    info: PluginInfo {
        name: c"File Explorer".as_ptr(),
        description: c"Browse local and remote files".as_ptr(),
        version: c"0.2.0".as_ptr(),
        plugin_type: PluginType::Panel,
        panel_location: PanelLocation::Left,
        dependencies: std::ptr::null(),
        num_dependencies: 0,
    },
    state: FilesPlugin,
    setup: |api| FilesPlugin::new(api),
    event: |state, event| state.handle_event(event),
    render: |state| state.render(),
    query: |state, method, args| state.handle_query(method, args),
);
