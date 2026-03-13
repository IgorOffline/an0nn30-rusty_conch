# conch-files

A native dual-pane file explorer plugin for Conch with upload/download support. The panel is split into two halves — the top pane always shows the local filesystem, and the bottom pane shows the remote (SFTP) filesystem when an SSH tab is active, or a second local view otherwise.

## Features

- **Dual-pane layout** — top pane (local) and bottom pane (remote/local) split the panel equally
- **File transfer** — upload (local → remote) and download (remote → local) buttons between panes
- **Local copy** — when both panes are local (no SSH session), transfer buttons perform a file copy
- **Auto-switching** — the bottom pane reacts to `ssh.session_ready`, `ssh.session_closed`, and `app.tab_changed` bus events to follow the focused session
- **Navigation** — each pane has independent back/forward history, home button, and path text input
- **Sorting** — click column headers to sort by name, extension, size, or modified date
- **Column visibility** — right-click column headers to toggle Ext/Size/Modified columns
- **Context menu** — right-click rows for New Folder, Rename, Delete, Copy Path

## Architecture

```
src/
  lib.rs      — plugin entry point, dual-pane orchestration, transfer logic
  pane.rs     — reusable single-pane file browser (navigation, events, rendering)
  local.rs    — local filesystem listing (std::fs::read_dir)
  remote.rs   — SFTP operations via query_plugin("SSH Manager", ...)
  format.rs   — file size, date, and extension formatting helpers
```

The plugin is a `cdylib` built with the `declare_plugin!` macro from `conch_plugin_sdk`. It registers a left-side panel and communicates with the host through:

- **Widget events** — button clicks, table interactions, toolbar input (prefixed per pane)
- **Bus events** — subscribes to `ssh.session_ready`, `ssh.session_closed`, `app.tab_changed`
- **Plugin queries** — calls `conch-ssh` SFTP services (`list_dir`, `read_file`, `write_file`, `mkdir`, `rename`, `delete`)

## Transfer flow

1. Select a file in one pane
2. Click the upload (↑) or download (↓) button
3. The file is transferred to the other pane's current directory
4. For SSH sessions: uses SFTP `read_file`/`write_file` via the SSH plugin
5. For local-only: uses `std::fs::copy`

## Dependencies

- `conch_plugin_sdk` — plugin ABI and widget types
- `conch-ssh` — provides SFTP operations for remote browsing (runtime dependency via plugin bus)
- `hostname` — resolves local machine name for the pane title
- `dirs` — resolves the user's home directory
- `base64` — encoding/decoding for SFTP file transfer
