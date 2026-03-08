# Conch Plugin Support for VS Code

Lua language support for [Conch](https://github.com/an0nn30/rusty_conch) terminal emulator plugins.

## Features

- **API Completions** — autocomplete for all Conch plugin globals (`session`, `app`, `ui`, `crypto`, `net`) with parameter hints and documentation
- **Hover Docs** — hover over any API function to see its signature and description
- **Diagnostics** — runs `conch check` on save to catch syntax errors, invalid API calls, and plugin header issues
- **Lifecycle Hints** — type information for panel plugin lifecycle functions (`setup`, `render`, `on_click`, `on_keybind`)

## Requirements

- [Lua Language Server](https://marketplace.visualstudio.com/items?itemName=sumneko.lua) (installed automatically as a dependency)
- `conch` CLI on your PATH (for diagnostics)

## Settings

| Setting | Default | Description |
|---------|---------|-------------|
| `conch.checkOnSave` | `true` | Run `conch check` on save |
| `conch.executablePath` | `"conch"` | Path to the `conch` executable |

## Development

```bash
cd editors/vscode
npm install
npm run compile
```

To test locally, press F5 in VS Code to launch an Extension Development Host.

## Packaging

```bash
npm run package   # produces conch-lua-0.1.0.vsix
```
