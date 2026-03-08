import * as vscode from "vscode";
import * as path from "path";
import { ConchDiagnostics } from "./diagnostics";

let diagnostics: ConchDiagnostics | undefined;

export function activate(context: vscode.ExtensionContext) {
  // Configure LuaLS to include our Conch API type definitions.
  configureLuaLS(context);

  // Set up diagnostics via `conch check`.
  diagnostics = new ConchDiagnostics();
  context.subscriptions.push(diagnostics);

  // Run diagnostics on save (if enabled).
  context.subscriptions.push(
    vscode.workspace.onDidSaveTextDocument((doc) => {
      if (doc.languageId === "lua" && isConchPlugin(doc)) {
        const config = vscode.workspace.getConfiguration("conch");
        if (config.get<boolean>("checkOnSave", true)) {
          diagnostics?.check(doc);
        }
      }
    })
  );

  // Run diagnostics when opening a Lua file.
  context.subscriptions.push(
    vscode.workspace.onDidOpenTextDocument((doc) => {
      if (doc.languageId === "lua" && isConchPlugin(doc)) {
        diagnostics?.check(doc);
      }
    })
  );

  // Clear diagnostics when closing a file.
  context.subscriptions.push(
    vscode.workspace.onDidCloseTextDocument((doc) => {
      diagnostics?.clear(doc.uri);
    })
  );

  // Register a manual check command.
  context.subscriptions.push(
    vscode.commands.registerCommand("conch.checkPlugin", () => {
      const editor = vscode.window.activeTextEditor;
      if (editor && editor.document.languageId === "lua") {
        diagnostics?.check(editor.document);
      }
    })
  );

  // Check all open Lua files on activation.
  for (const doc of vscode.workspace.textDocuments) {
    if (doc.languageId === "lua" && isConchPlugin(doc)) {
      diagnostics.check(doc);
    }
  }
}

export function deactivate() {
  diagnostics?.dispose();
}

/**
 * Configure LuaLS (sumneko.lua) to include Conch API type definitions.
 *
 * Adds our `lua/` directory to the workspace library so LuaLS picks up
 * the EmmyLua annotations in `conch.lua`.
 */
function configureLuaLS(context: vscode.ExtensionContext) {
  const libraryPath = path.join(context.extensionPath, "lua");

  const luaConfig = vscode.workspace.getConfiguration("Lua");

  // Get existing workspace libraries.
  const libraries: string[] =
    luaConfig.get<string[]>("workspace.library") || [];

  // Add our library path if not already present.
  if (!libraries.includes(libraryPath)) {
    libraries.push(libraryPath);
    luaConfig.update(
      "workspace.library",
      libraries,
      vscode.ConfigurationTarget.Workspace
    );
  }

  // Suppress the "undefined global" warning for our API globals.
  const globals: string[] =
    luaConfig.get<string[]>("diagnostics.globals") || [];
  const conchGlobals = [
    "session",
    "app",
    "ui",
    "crypto",
    "net",
    "setup",
    "render",
    "on_click",
    "on_keybind",
  ];

  let changed = false;
  for (const g of conchGlobals) {
    if (!globals.includes(g)) {
      globals.push(g);
      changed = true;
    }
  }
  if (changed) {
    luaConfig.update(
      "diagnostics.globals",
      globals,
      vscode.ConfigurationTarget.Workspace
    );
  }
}

/**
 * Heuristic to detect whether a Lua file is a Conch plugin.
 *
 * Checks for plugin header comments or usage of Conch API globals.
 */
function isConchPlugin(doc: vscode.TextDocument): boolean {
  const text = doc.getText(new vscode.Range(0, 0, 20, 0));
  return (
    text.includes("-- plugin-name:") ||
    text.includes("-- plugin-type:") ||
    text.includes("-- plugin-description:") ||
    /\b(session|app|ui|crypto|net)\.\w+/.test(text)
  );
}
