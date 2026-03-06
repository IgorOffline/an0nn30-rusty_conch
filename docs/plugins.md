# Conch Plugin System

Conch includes a Lua 5.4 plugin system that lets you automate tasks, build tools, and extend the terminal with custom functionality. Plugins run in a sandboxed environment with access to sessions, UI dialogs, cryptography, and application controls.

## Getting Started

### Plugin Location

Place `.lua` files in the plugins directory:

```
~/.config/conch/plugins/
```

Conch scans this directory on startup and when you click Refresh in the Plugins panel.

### Plugin Header

Every plugin should start with metadata comments:

```lua
-- plugin-name: My Plugin
-- plugin-description: A short description of what it does
-- plugin-version: 1.0.0
```

These comments must be at the top of the file (before any code). The `plugin-name` and `plugin-description` appear in the sidebar's Plugins panel.

### Running Plugins

- Open the left sidebar and switch to the **Plugins** tab
- Select a plugin and click **Run**, or double-click it
- Use **Cmd+Shift+P** (configurable) to search and run plugins by name without leaving the keyboard

### LuaRocks Modules

Plugins can `require()` LuaRocks modules installed to:

```
~/.config/conch/lua_modules/
```

Install modules with:

```bash
luarocks --tree ~/.config/conch/lua_modules install <module-name>
```

Plugins can also `require()` other `.lua` files from their own directory.

## API Reference

Conch exposes four global tables to plugins: `session`, `app`, `ui`, and `crypto`.

### `session` — Terminal Session Interaction

| Function | Returns | Description |
|----------|---------|-------------|
| `session.exec(cmd)` | `string` | Execute a command on the active session and return output |
| `session.send(text)` | — | Send raw text to the active session (no newline) |
| `session.run(cmd)` | — | Send a command + newline to the active session |
| `session.current()` | `table\|nil` | Get info about the active session |
| `session.all()` | `table` | Get info about all open sessions |
| `session.named(name)` | `table\|nil` | Get a handle to a session by name |

#### Session Info Table

The tables returned by `session.current()`, `session.all()`, and `session.named()` contain:

| Field | Type | Description |
|-------|------|-------------|
| `id` | `string` | Unique session identifier |
| `title` | `string` | Session display title |
| `type` | `string` | `"local"` or `"ssh"` |

#### Named Session Handles

`session.named(name)` returns a handle table with bound methods that target that specific session:

```lua
local srv = session.named("webserver")
if srv then
    srv.run("uptime")          -- runs on "webserver", not the active tab
    srv.send("ls -la\n")       -- send raw text
    local out = srv.exec("hostname")
end
```

### `app` — Application Controls

| Function | Returns | Description |
|----------|---------|-------------|
| `app.open_session(name)` | — | Open a saved SSH connection by server name or host |
| `app.clipboard(text)` | — | Copy text to the system clipboard |
| `app.notify(msg)` | — | Show a notification |
| `app.log(msg)` | — | Log a message (visible in application logs) |
| `app.servers()` | `table` | Get a list of all configured server names |

### `ui` — User Interface

#### Output Panel

| Function | Returns | Description |
|----------|---------|-------------|
| `ui.append(text)` | — | Append a line to the plugin output panel in the sidebar |
| `ui.clear()` | — | Clear the plugin output panel |

#### Dialogs

All dialog functions are **blocking** — the plugin pauses until the user responds.

| Function | Returns | Description |
|----------|---------|-------------|
| `ui.form(title, fields)` | `table\|nil` | Show a form dialog; returns field values or `nil` if cancelled |
| `ui.prompt(message)` | `string\|nil` | Show a text input prompt |
| `ui.confirm(message)` | `boolean` | Show a Yes/No confirmation dialog |
| `ui.alert(title, message)` | — | Show an informational alert |
| `ui.error(title, message)` | — | Show an error alert (red text) |
| `ui.show(title, text)` | — | Show a read-only text viewer with a Copy button |
| `ui.table(title, columns, rows)` | — | Show a table viewer |

#### Progress Indicator

| Function | Returns | Description |
|----------|---------|-------------|
| `ui.progress(message)` | — | Show a progress spinner with a message |
| `ui.hide_progress()` | — | Hide the progress spinner |

#### Form Fields

The `ui.form()` function accepts a table of field descriptors. Each field is a table with a `type` key:

| Type | Keys | Description |
|------|------|-------------|
| `"text"` | `name`, `label`, `default` | Single-line text input |
| `"password"` | `name`, `label` | Password input (masked) |
| `"combo"` | `name`, `label`, `options`, `default` | Dropdown select |
| `"checkbox"` | `name`, `label`, `default` | Boolean checkbox |
| `"separator"` | — | Visual separator line |
| `"label"` | `text` | Static text (italic, not editable) |

The return value is a table mapping field `name` to the user's input (strings for all types, `"true"`/`"false"` for checkboxes), or `nil` if the user cancelled.

```lua
local result = ui.form("Settings", {
    { type = "text",     name = "host",     label = "Hostname",  default = "localhost" },
    { type = "text",     name = "port",     label = "Port",      default = "8080" },
    { type = "combo",    name = "protocol", label = "Protocol",  options = {"HTTP", "HTTPS"}, default = "HTTPS" },
    { type = "checkbox", name = "verbose",  label = "Verbose",   default = false },
    { type = "separator" },
    { type = "label",    text = "Leave blank to use defaults" },
    { type = "password", name = "token",    label = "API Token" },
})

if result then
    print(result.host)       -- "localhost"
    print(result.protocol)   -- "HTTPS"
    print(result.verbose)    -- "true" or "false"
end
```

### `crypto` — Cryptography

AES encryption and decryption with PBKDF2 key derivation. All functions run on a background thread to avoid blocking the UI.

| Function | Returns | Description |
|----------|---------|-------------|
| `crypto.encrypt(plaintext, passphrase, algorithm)` | `string` | Encrypt text, returns base64-encoded ciphertext |
| `crypto.decrypt(encoded, passphrase, algorithm)` | `string` | Decrypt base64-encoded ciphertext, returns plaintext |
| `crypto.algorithms()` | `table` | List supported algorithm strings |

#### Supported Algorithms

- `AES-128-CBC`, `AES-256-CBC` — CBC mode with PKCS7 padding
- `AES-128-GCM`, `AES-256-GCM` — GCM mode (authenticated encryption, recommended)
- `AES-128-ECB`, `AES-256-ECB` — ECB mode (not recommended for most uses)

Key derivation uses PBKDF2-HMAC-SHA256 with 310,000 iterations and a random 16-byte salt. The output format is `base64(salt || iv || ciphertext)`.

## Sandboxing

Plugins run in a restricted Lua environment. The following standard library modules are **removed** for safety:

- `os` — no file system operations or process execution
- `io` — no file I/O
- `loadfile` / `dofile` — no arbitrary file execution

The `require()` function is available but restricted to the plugin's own directory and the LuaRocks module path.

## Example: Encrypt / Decrypt Plugin

```lua
-- plugin-name: Encrypt / Decrypt
-- plugin-description: AES encryption (CBC, GCM, ECB) with PBKDF2 key derivation
-- plugin-version: 2.0.0

local ALGORITHMS = crypto.algorithms()

local vals = ui.form("Encrypt / Decrypt", {
    { type = "combo",    name = "mode",      label = "Mode",       options = { "Encrypt", "Decrypt" }, default = "Encrypt" },
    { type = "combo",    name = "algorithm",  label = "Algorithm",  options = ALGORITHMS, default = "AES-256-GCM" },
    { type = "password", name = "key",        label = "Passphrase" },
    { type = "separator" },
    { type = "label",    text = "GCM is recommended (authenticated encryption)" },
    { type = "text",     name = "input",      label = "Input",      default = "" },
})

if not vals then return end

if not vals.key or vals.key == "" then
    ui.error("Error", "Passphrase must not be empty.")
    return
end

if not vals.input or vals.input == "" then
    ui.error("Error", "Input text must not be empty.")
    return
end

local ok, result = pcall(function()
    if vals.mode == "Encrypt" then
        return crypto.encrypt(vals.input, vals.key, vals.algorithm)
    else
        return crypto.decrypt(vals.input, vals.key, vals.algorithm)
    end
end)

if ok and result then
    ui.show(vals.mode .. "ed (" .. vals.algorithm .. ")", result)
    app.clipboard(result)
    ui.append(vals.mode .. " complete (" .. vals.algorithm .. "). Result copied to clipboard.")
else
    ui.error(vals.mode .. " Failed", tostring(result))
end
```

## Example: Multi-Session Deploy

```lua
-- plugin-name: Deploy to Servers
-- plugin-description: Run a deploy command across multiple sessions
-- plugin-version: 1.0.0

local servers = app.servers()
if #servers == 0 then
    ui.error("No Servers", "No saved servers found.")
    return
end

local vals = ui.form("Deploy", {
    { type = "text", name = "command", label = "Command", default = "cd /app && git pull && systemctl restart app" },
})
if not vals then return end

local confirmed = ui.confirm("Run on all open sessions?\n\nCommand: " .. vals.command)
if not confirmed then return end

ui.clear()
local sessions = session.all()
for _, s in ipairs(sessions) do
    if s.type == "ssh" then
        ui.append("Deploying to " .. s.title .. "...")
        local handle = session.named(s.title)
        if handle then
            handle.run(vals.command)
            ui.append("  Sent to " .. s.title)
        end
    end
end
ui.append("Deploy complete.")
```

## Example: Quick Connect

```lua
-- plugin-name: Quick Connect
-- plugin-description: Connect to a server from a searchable list
-- plugin-version: 1.0.0

local servers = app.servers()
if #servers == 0 then
    ui.alert("No Servers", "No saved servers configured.")
    return
end

-- Build options table for combo box
local result = ui.form("Quick Connect", {
    { type = "combo", name = "server", label = "Server", options = servers, default = servers[1] },
})

if result then
    app.open_session(result.server)
    ui.append("Connecting to " .. result.server .. "...")
end
```
