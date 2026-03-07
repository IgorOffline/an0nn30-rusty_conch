//! SSH connection initiation, tunnel activation, and connecting screen UI.

use std::time::Instant;

use conch_core::models::SavedTunnel;
use conch_session::SshSession;
use uuid::Uuid;

use crate::app::{ConchApp, PendingSsh, PendingSshInfo, DEFAULT_COLS, DEFAULT_ROWS};
use crate::sessions::build_term_config;

impl ConchApp {
    /// Spawn an async SSH connection attempt on the tokio runtime.
    pub(crate) fn start_ssh_connect(
        &mut self,
        host: String,
        port: u16,
        user: String,
        identity_file: Option<String>,
        proxy_command: Option<String>,
        proxy_jump: Option<String>,
        password: Option<String>,
    ) {
        let id = Uuid::new_v4();
        let (tx, rx) = std::sync::mpsc::channel();

        let label = host.clone();
        let detail = format!("{user}@{host}:{port}");
        self.pending_ssh_info.insert(id, PendingSshInfo {
            label,
            detail,
            started: Instant::now(),
        });

        self.state.tab_order.push(id);
        self.state.active_tab = Some(id);

        let host_clone = host.clone();
        let term_config = build_term_config(&self.state.user_config.terminal.cursor);
        self.rt.spawn(async move {
            let params = conch_session::ConnectParams {
                host: host_clone,
                port,
                user,
                identity_file: identity_file.map(std::path::PathBuf::from),
                password,
                proxy_command,
                proxy_jump,
            };
            let result = SshSession::connect(&params, DEFAULT_COLS, DEFAULT_ROWS, term_config)
                .await
                .map_err(|e| format!("{host}: {e}"));
            let _ = tx.send(result);
        });

        self.pending_ssh_connections.push(PendingSsh { id, rx });
    }

    /// Collect all SSH server entries from sidebar folders + ssh_config hosts.
    pub(crate) fn collect_all_servers(&self) -> Vec<conch_core::models::ServerEntry> {
        let mut servers = Vec::new();
        fn collect_from_folders(
            folders: &[conch_core::models::ServerFolder],
            out: &mut Vec<conch_core::models::ServerEntry>,
        ) {
            for folder in folders {
                out.extend(folder.servers.iter().cloned());
                collect_from_folders(&folder.subfolders, out);
            }
        }
        collect_from_folders(&self.state.sessions_config.folders, &mut servers);
        for host in &self.state.ssh_config_hosts {
            if !servers.iter().any(|s| s.session_key() == host.session_key()) {
                servers.push(host.clone());
            }
        }
        servers
    }

    /// Kick off async tunnel activation (SSH connect + port forward).
    pub(crate) fn activate_tunnel(&mut self, tunnel: &SavedTunnel) {
        let tunnel = tunnel.clone();
        let servers = self.collect_all_servers();
        log::info!(
            "activate_tunnel: looking for session_key='{}' among {} servers",
            tunnel.session_key,
            servers.len(),
        );
        for s in &servers {
            log::debug!("  available server: '{}' key='{}'", s.name, s.session_key());
        }
        let server = servers.into_iter().find(|s| s.session_key() == tunnel.session_key);
        let Some(server) = server else {
            log::error!(
                "No matching server for tunnel session_key '{}'. \
                 Check that the server is configured in the sidebar or ssh_config.",
                tunnel.session_key,
            );
            return;
        };
        log::info!(
            "activate_tunnel: matched server '{}' ({}@{}:{}), connecting for tunnel {} (:{} -> {}:{})",
            server.name, server.user, server.host, server.port,
            tunnel.id, tunnel.local_port, tunnel.remote_host, tunnel.remote_port,
        );
        let tm = self.tunnel_manager.clone_inner();
        let (tx, rx) = std::sync::mpsc::channel();
        self.pending_tunnel_results.push((tunnel.id, rx));

        self.rt.spawn(async move {
            let params = conch_session::ConnectParams::from(&server);
            log::info!(
                "activate_tunnel[{}]: SSH connecting to {}@{}:{} ...",
                tunnel.id, params.user, params.host, params.port,
            );
            let result = async {
                let handle = conch_session::connect_tunnel(&params).await
                    .map_err(|e| format!("SSH connect failed for {}@{}:{}: {e}", params.user, params.host, params.port))?;
                log::info!(
                    "activate_tunnel[{}]: SSH connected, starting local forward 127.0.0.1:{} -> {}:{} ...",
                    tunnel.id, tunnel.local_port, tunnel.remote_host, tunnel.remote_port,
                );
                tm.start_local_forward(
                    tunnel.id,
                    handle,
                    tunnel.local_port,
                    tunnel.remote_host.clone(),
                    tunnel.remote_port,
                ).await.map_err(|e| format!("Port forward failed: {e}"))
            }.await;
            match &result {
                Ok(()) => log::info!("activate_tunnel[{}]: tunnel active and listening", tunnel.id),
                Err(e) => log::error!("activate_tunnel[{}]: failed: {e}", tunnel.id),
            }
            let _ = tx.send(result);
        });
    }
}

/// Render the "Connecting to..." screen with a bouncing progress indicator.
pub(crate) fn show_connecting_screen(ui: &mut egui::Ui, info: &PendingSshInfo) {
    let rect = ui.available_rect_before_wrap();

    let bg = if ui.visuals().dark_mode {
        egui::Color32::from_gray(30)
    } else {
        egui::Color32::from_gray(241)
    };
    ui.painter().rect_filled(rect, 0.0, bg);

    let center = rect.center();

    let heading = format!("Connecting to {}\u{2026}", info.label);
    let heading_galley = ui.painter().layout_no_wrap(
        heading,
        egui::FontId::new(28.0, egui::FontFamily::Proportional),
        if ui.visuals().dark_mode { egui::Color32::WHITE } else { egui::Color32::BLACK },
    );
    let heading_pos = egui::Pos2::new(
        center.x - heading_galley.size().x / 2.0,
        center.y - 40.0,
    );
    ui.painter().galley(heading_pos, heading_galley, egui::Color32::PLACEHOLDER);

    let detail_galley = ui.painter().layout_no_wrap(
        info.detail.clone(),
        egui::FontId::new(16.0, egui::FontFamily::Proportional),
        if ui.visuals().dark_mode { egui::Color32::from_gray(200) } else { egui::Color32::from_gray(40) },
    );
    let detail_pos = egui::Pos2::new(
        center.x - detail_galley.size().x / 2.0,
        center.y + 5.0,
    );
    ui.painter().galley(detail_pos, detail_galley, egui::Color32::PLACEHOLDER);

    // Bouncing progress bar.
    let bar_w = 400.0_f32.min(rect.width() * 0.6);
    let bar_h = 6.0;
    let bar_y = center.y + 50.0;
    let bar_rect = egui::Rect::from_min_size(
        egui::Pos2::new(center.x - bar_w / 2.0, bar_y),
        egui::Vec2::new(bar_w, bar_h),
    );

    let track_color = if ui.visuals().dark_mode {
        egui::Color32::from_gray(60)
    } else {
        egui::Color32::from_gray(210)
    };
    ui.painter().rect_filled(bar_rect, bar_h / 2.0, track_color);

    let elapsed = info.started.elapsed().as_secs_f32();
    let cycle = 1.8;
    let t = (elapsed % cycle) / cycle;
    let pos_t = if t < 0.5 { t * 2.0 } else { 2.0 - t * 2.0 };
    let eased = pos_t * pos_t * (3.0 - 2.0 * pos_t);
    let indicator_w = bar_w * 0.15;
    let indicator_x = bar_rect.min.x + eased * (bar_w - indicator_w);
    let indicator_rect = egui::Rect::from_min_size(
        egui::Pos2::new(indicator_x, bar_y),
        egui::Vec2::new(indicator_w, bar_h),
    );
    let accent = egui::Color32::from_rgb(66, 133, 244);
    ui.painter().rect_filled(indicator_rect, bar_h / 2.0, accent);
}
