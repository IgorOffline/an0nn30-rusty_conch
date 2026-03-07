//! Plugin system: discovery, lifecycle, and command handling.

use std::collections::HashSet;
use std::path::PathBuf;

use conch_core::config;
use conch_plugin::{
    PluginCommand, PluginContext, PluginMeta, PluginResponse, SessionInfoData,
    SessionTarget, discover_plugins, run_plugin,
};

use crate::app::{ConchApp, RunningPlugin};
use crate::state::SessionBackend;
use crate::ui::dialogs::plugin_dialog::{ActivePluginDialog, FormFieldState};

impl ConchApp {
    /// Drain commands from all running plugins and handle them.
    pub(crate) fn poll_plugin_events(&mut self, ctx: &egui::Context) {
        let mut immediate_cmds = Vec::new();
        self.running_plugins.retain_mut(|rp| {
            loop {
                match rp.commands_rx.try_recv() {
                    Ok((cmd, resp_tx)) => {
                        if is_dialog_command(&cmd) {
                            rp.pending_dialogs.push((cmd, resp_tx));
                        } else {
                            immediate_cmds.push((cmd, resp_tx));
                        }
                        ctx.request_repaint();
                    }
                    Err(tokio::sync::mpsc::error::TryRecvError::Empty) => return true,
                    Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => return false,
                }
            }
        });

        for (cmd, resp_tx) in immediate_cmds {
            self.handle_plugin_command(cmd, resp_tx);
        }

        if self.active_plugin_dialog.is_none() {
            for rp in &mut self.running_plugins {
                if !rp.pending_dialogs.is_empty() {
                    let (cmd, resp_tx) = rp.pending_dialogs.remove(0);
                    self.promote_dialog_command(cmd, resp_tx);
                    break;
                }
            }
        }
    }

    /// Handle a non-dialog plugin command immediately.
    pub(crate) fn handle_plugin_command(
        &mut self,
        cmd: PluginCommand,
        resp_tx: tokio::sync::mpsc::UnboundedSender<PluginResponse>,
    ) {
        match cmd {
            PluginCommand::Send { target, text } => {
                if let Some(session) = self.resolve_session(&target) {
                    session.backend.write(text.as_bytes());
                }
                let _ = resp_tx.send(PluginResponse::Ok);
            }
            PluginCommand::Exec { target, command } => {
                if let Some(session) = self.resolve_session(&target) {
                    session.backend.write(format!("{}\n", command).as_bytes());
                }
                let _ = resp_tx.send(PluginResponse::Output(String::new()));
            }
            PluginCommand::OpenSession { name } => {
                let servers = self.collect_all_servers();
                if let Some(server) = servers.iter().find(|s| s.name == name || s.host == name) {
                    self.start_ssh_connect(
                        server.host.clone(),
                        server.port,
                        server.user.clone(),
                        server.identity_file.clone(),
                        server.proxy_command.clone(),
                        server.proxy_jump.clone(),
                        None,
                    );
                }
                let _ = resp_tx.send(PluginResponse::Ok);
            }
            PluginCommand::Clipboard(text) => {
                self.pending_clipboard = Some(text);
                let _ = resp_tx.send(PluginResponse::Ok);
            }
            PluginCommand::Notify(msg) => {
                log::info!("[plugin] {msg}");
                let _ = resp_tx.send(PluginResponse::Ok);
            }
            PluginCommand::Log(msg) => {
                log::info!("[plugin] {msg}");
                let _ = resp_tx.send(PluginResponse::Ok);
            }
            PluginCommand::UiAppend(text) => {
                self.plugin_output_lines.push(text);
                let _ = resp_tx.send(PluginResponse::Ok);
            }
            PluginCommand::UiClear => {
                self.plugin_output_lines.clear();
                let _ = resp_tx.send(PluginResponse::Ok);
            }
            PluginCommand::GetCurrentSession => {
                let info = self.state.active_tab.and_then(|id| {
                    self.state.sessions.get(&id).map(|s| SessionInfoData {
                        id: id.to_string(),
                        title: s.custom_title.as_ref().unwrap_or(&s.title).clone(),
                        session_type: match &s.backend {
                            SessionBackend::Local(_) => "local".into(),
                            SessionBackend::Ssh(_) => "ssh".into(),
                        },
                    })
                });
                let _ = resp_tx.send(PluginResponse::SessionInfo(info));
            }
            PluginCommand::GetAllSessions => {
                let list: Vec<SessionInfoData> = self
                    .state
                    .sessions
                    .iter()
                    .map(|(id, s)| SessionInfoData {
                        id: id.to_string(),
                        title: s.custom_title.as_ref().unwrap_or(&s.title).clone(),
                        session_type: match &s.backend {
                            SessionBackend::Local(_) => "local".into(),
                            SessionBackend::Ssh(_) => "ssh".into(),
                        },
                    })
                    .collect();
                let _ = resp_tx.send(PluginResponse::SessionList(list));
            }
            PluginCommand::GetNamedSession { name } => {
                let info = self.state.sessions.iter().find_map(|(id, s)| {
                    let title = s.custom_title.as_ref().unwrap_or(&s.title);
                    if title == &name {
                        Some(SessionInfoData {
                            id: id.to_string(),
                            title: title.clone(),
                            session_type: match &s.backend {
                                SessionBackend::Local(_) => "local".into(),
                                SessionBackend::Ssh(_) => "ssh".into(),
                            },
                        })
                    } else {
                        None
                    }
                });
                let _ = resp_tx.send(PluginResponse::SessionInfo(info));
            }
            PluginCommand::GetServers => {
                let names: Vec<String> = self
                    .collect_all_servers()
                    .iter()
                    .map(|s| s.name.clone())
                    .collect();
                let _ = resp_tx.send(PluginResponse::ServerList(names));
            }
            PluginCommand::ShowProgress { message } => {
                self.plugin_progress = Some(message);
                let _ = resp_tx.send(PluginResponse::Ok);
            }
            PluginCommand::HideProgress => {
                self.plugin_progress = None;
                let _ = resp_tx.send(PluginResponse::Ok);
            }
            _ => {
                let _ = resp_tx.send(PluginResponse::Ok);
            }
        }
    }

    /// Promote a dialog command into the active dialog slot.
    pub(crate) fn promote_dialog_command(
        &mut self,
        cmd: PluginCommand,
        resp_tx: tokio::sync::mpsc::UnboundedSender<PluginResponse>,
    ) {
        let dialog = match cmd {
            PluginCommand::ShowForm { title, fields } => {
                let field_states: Vec<FormFieldState> =
                    fields.iter().map(FormFieldState::from_field).collect();
                ActivePluginDialog::Form {
                    title,
                    fields: field_states,
                    resp_tx,
                    focus_first: true,
                }
            }
            PluginCommand::ShowPrompt { message } => ActivePluginDialog::Prompt {
                message,
                input: String::new(),
                resp_tx,
                focus_first: true,
            },
            PluginCommand::ShowConfirm { message } => ActivePluginDialog::Confirm {
                message,
                resp_tx,
            },
            PluginCommand::ShowAlert { title, message } => ActivePluginDialog::Alert {
                title,
                message,
                resp_tx,
            },
            PluginCommand::ShowError { title, message } => ActivePluginDialog::Error {
                title,
                message,
                resp_tx,
            },
            PluginCommand::ShowText { title, text } => ActivePluginDialog::Text {
                title,
                text,
                copied_at: None,
                resp_tx,
            },
            PluginCommand::ShowTable {
                title,
                columns,
                rows,
            } => ActivePluginDialog::Table {
                title,
                columns,
                rows,
                resp_tx,
            },
            _ => return,
        };
        self.active_plugin_dialog = Some(dialog);
    }

    /// Resolve a session target to a `&Session`.
    pub(crate) fn resolve_session(&self, target: &SessionTarget) -> Option<&crate::state::Session> {
        match target {
            SessionTarget::Current => {
                self.state.active_tab.and_then(|id| self.state.sessions.get(&id))
            }
            SessionTarget::Named(name) => {
                self.state.sessions.values().find(|s| {
                    let title = s.custom_title.as_ref().unwrap_or(&s.title);
                    title == name
                })
            }
        }
    }

    /// Launch a discovered plugin by its index in `discovered_plugins`.
    pub(crate) fn run_plugin_by_index(&mut self, idx: usize) {
        let Some(meta) = self.discovered_plugins.get(idx).cloned() else {
            return;
        };
        let (ctx, commands_rx) = PluginContext::new();
        let path = meta.path.clone();
        self.rt.spawn(async move {
            if let Err(e) = run_plugin(&path, ctx).await {
                log::error!("Plugin '{}' failed: {e}", path.display());
            }
        });
        self.running_plugins.push(RunningPlugin {
            meta,
            commands_rx,
            pending_dialogs: Vec::new(),
        });
    }

    /// Stop a running plugin by index.
    pub(crate) fn stop_plugin(&mut self, idx: usize) {
        if idx < self.running_plugins.len() {
            self.running_plugins.remove(idx);
        }
    }

    /// Re-scan the plugins directory.
    pub(crate) fn refresh_plugins(&mut self) {
        self.discovered_plugins = scan_plugin_dirs();
    }
}

pub(crate) fn is_dialog_command(cmd: &PluginCommand) -> bool {
    matches!(
        cmd,
        PluginCommand::ShowForm { .. }
            | PluginCommand::ShowPrompt { .. }
            | PluginCommand::ShowConfirm { .. }
            | PluginCommand::ShowAlert { .. }
            | PluginCommand::ShowError { .. }
            | PluginCommand::ShowText { .. }
            | PluginCommand::ShowTable { .. }
    )
}

/// Scan for plugins in the native config dir and the legacy `~/.config/conch/` dir.
pub(crate) fn scan_plugin_dirs() -> Vec<PluginMeta> {
    let mut plugins = Vec::new();
    let mut seen_names = HashSet::new();

    let native_dir = config::config_dir().join("plugins");
    if let Ok(found) = discover_plugins(&native_dir) {
        for p in found {
            let key = p.path.file_name().unwrap_or_default().to_string_lossy().into_owned();
            seen_names.insert(key);
            plugins.push(p);
        }
    }

    if let Some(home) = std::env::var_os("HOME") {
        let legacy_dir = PathBuf::from(home).join(".config/conch/plugins");
        if legacy_dir != native_dir {
            if let Ok(found) = discover_plugins(&legacy_dir) {
                for p in found {
                    let key = p.path.file_name().unwrap_or_default().to_string_lossy().into_owned();
                    if !seen_names.contains(&key) {
                        seen_names.insert(key);
                        plugins.push(p);
                    }
                }
            }
        }
    }

    plugins
}

/// Send a cancel/close response for a plugin dialog so the plugin coroutine doesn't hang.
pub(crate) fn send_plugin_dialog_cancel(dialog: &ActivePluginDialog) {
    match dialog {
        ActivePluginDialog::Form { resp_tx, .. } => {
            let _ = resp_tx.send(PluginResponse::FormResult(None));
        }
        ActivePluginDialog::Prompt { resp_tx, .. } => {
            let _ = resp_tx.send(PluginResponse::Ok);
        }
        ActivePluginDialog::Confirm { resp_tx, .. } => {
            let _ = resp_tx.send(PluginResponse::Bool(false));
        }
        ActivePluginDialog::Alert { resp_tx, .. }
        | ActivePluginDialog::Error { resp_tx, .. }
        | ActivePluginDialog::Text { resp_tx, .. }
        | ActivePluginDialog::Table { resp_tx, .. } => {
            let _ = resp_tx.send(PluginResponse::Ok);
        }
    }
}
