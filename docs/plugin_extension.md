# Plugin System Extension: Panel Plugins

## Overview

Extend the plugin system so Lua plugins can register persistent UI panels
in the sidebar, not just run-once actions. Panel plugins describe their UI
declaratively and the Rust side renders it with egui.

---

## Phase 1: Sidebar Panels (complete)

**Status:** Fully implemented.

Panel plugins declare `plugin-type: panel` and get their own sidebar tab with
a declarative widget set and a periodic refresh loop.

### Features implemented

- Plugin header metadata: `plugin-name`, `plugin-description`, `plugin-version`, `plugin-type`, `plugin-icon`, `plugin-keybind`
- Lifecycle: `setup()`, `render()`, `on_click(button_id)`, `on_keybind(action)`
- Declarative widget API: heading, text, label, separator, table, progress, button, key-value
- Silent command execution via separate SSH channels / local subprocesses
- Platform detection (`session.platform()`)
- Custom plugin icons with validation (extension, size, magic bytes, decode)
- Plugin keybindings with priority resolution and config overrides
- Event polling: buttons and keybinds dispatched to plugin handlers between render cycles
- Five API modules: `session`, `app`, `ui`, `crypto`, `net`
- Networking API: TCP port scanning, DNS resolution, timing
- Load/unload persistence in `state.toml`
- Three example plugins: System Info, Port Scanner, Encrypt/Decrypt

---

## Phase 2: Interactive Widgets

**Goal:** Add interactive widgets to panels — combo selectors, text inputs,
toggle switches — with event callbacks flowing back to the plugin.

### Additional widgets

| Function | Description |
|----------|-------------|
| `ui.panel_combo(id, label, options, default)` | Dropdown selector |
| `ui.panel_text_input(id, label, default)` | Editable text field |
| `ui.panel_toggle(id, label, default)` | On/off toggle |
| `ui.panel_color(label, r, g, b)` | Colored status indicator |

### Callback model

```lua
function on_change(widget_id, value)
    -- Called when a combo, text_input, or toggle changes
end
```

Widget state is stored Rust-side so the panel retains interactive state
between render cycles. Changes are sent back via
`PluginCommand::PanelWidgetChanged { id, value }`.

---

## Phase 3: Bottom Panels

**Goal:** Allow plugins to create dockable panels below the terminal area,
suitable for log tailing, build output, and persistent status displays.

### Panel placement

```lua
-- plugin-placement: bottom
```

- Bottom panels render as a resizable horizontal strip under the terminal.
- Multiple bottom panels are shown as tabs within the strip.
- The same declarative widget set from Phase 1/2 applies.
- Adds a `ui.panel_scroll_text(lines)` widget optimized for streaming output.

### Architecture

- `SidebarTab` concept extended with a `BottomPanel` area.
- Bottom panel height is user-resizable and persisted.
- A panel can be dragged between sidebar and bottom positions.
