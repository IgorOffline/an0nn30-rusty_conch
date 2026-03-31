//! Permission-checked HostApi wrapper.
//!
//! Phase 2 scaffolding:
//! - Plugins declare requested capabilities.
//! - Host checks capability before each sensitive HostApi call.
//! - By default, denied checks are logged but allowed (compat mode).
//! - With `plugin_permissions_enforce`, denied checks are blocked.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use conch_plugin::HostApi;
use conch_plugin_sdk::PanelLocation;
use parking_lot::RwLock;

#[derive(Clone, Debug)]
pub(crate) struct PermissionProfile {
    pub allow_all: bool,
    pub allowed: HashSet<String>,
}

impl PermissionProfile {
    pub fn legacy_allow_all() -> Self {
        Self {
            allow_all: true,
            allowed: HashSet::new(),
        }
    }

    pub fn from_declared(declared: &[String]) -> Self {
        let mut allowed = HashSet::new();
        for p in declared {
            let norm = p.trim().to_ascii_lowercase();
            if !norm.is_empty() {
                allowed.insert(norm);
            }
        }
        Self {
            allow_all: false,
            allowed,
        }
    }
}

pub(crate) struct PermissionCheckedHostApi {
    inner: Arc<dyn HostApi>,
    base_plugin_name: String,
    profiles: Arc<RwLock<HashMap<String, PermissionProfile>>>,
}

impl PermissionCheckedHostApi {
    pub fn new(
        inner: Arc<dyn HostApi>,
        base_plugin_name: String,
        profiles: Arc<RwLock<HashMap<String, PermissionProfile>>>,
    ) -> Self {
        Self {
            inner,
            base_plugin_name,
            profiles,
        }
    }

    fn effective_plugin_name(&self) -> String {
        if self.base_plugin_name == "java" {
            if let Some(thread_name) = std::thread::current().name() {
                if let Some(name) = thread_name.strip_prefix("plugin:") {
                    return name.to_string();
                }
            }
        }
        self.base_plugin_name.clone()
    }

    fn check_capability(&self, method: &str, capability: &str) -> bool {
        let plugin_name = self.effective_plugin_name();
        let profiles = self.profiles.read();
        let profile = profiles
            .get(&plugin_name)
            .or_else(|| profiles.get(&self.base_plugin_name));

        if let Some(p) = profile {
            if p.allow_all || p.allowed.contains(capability) {
                return true;
            }
        } else {
            // Missing profile: keep compat behavior for now.
            return true;
        }

        let enforce = cfg!(feature = "plugin_permissions_enforce");
        if enforce {
            log::warn!(
                "[plugin:{}] denied '{}' (missing capability '{}')",
                plugin_name,
                method,
                capability
            );
            false
        } else {
            log::warn!(
                "[plugin:{}] missing capability '{}' for '{}' (compat mode: allowing)",
                plugin_name,
                capability,
                method
            );
            true
        }
    }
}

impl HostApi for PermissionCheckedHostApi {
    fn plugin_name(&self) -> &str {
        self.inner.plugin_name()
    }

    fn register_panel(&self, location: PanelLocation, name: &str, icon: Option<&str>) -> u64 {
        if !self.check_capability("register_panel", "ui.panel") {
            return 0;
        }
        self.inner.register_panel(location, name, icon)
    }

    fn set_widgets(&self, handle: u64, widgets_json: &str) {
        if !self.check_capability("set_widgets", "ui.panel") {
            return;
        }
        self.inner.set_widgets(handle, widgets_json)
    }

    fn log(&self, level: u8, msg: &str) {
        self.inner.log(level, msg)
    }

    fn notify(&self, json: &str) {
        if !self.check_capability("notify", "ui.notify") {
            return;
        }
        self.inner.notify(json)
    }

    fn set_status(&self, text: Option<&str>, level: u8, progress: f32) {
        if !self.check_capability("set_status", "ui.notify") {
            return;
        }
        self.inner.set_status(text, level, progress)
    }

    fn publish_event(&self, event_type: &str, data_json: &str) {
        if !self.check_capability("publish_event", "bus.publish") {
            return;
        }
        self.inner.publish_event(event_type, data_json)
    }

    fn subscribe(&self, event_type: &str) {
        if !self.check_capability("subscribe", "bus.subscribe") {
            return;
        }
        self.inner.subscribe(event_type)
    }

    fn query_plugin(&self, target: &str, method: &str, args_json: &str) -> Option<String> {
        if !self.check_capability("query_plugin", "bus.query") {
            return None;
        }
        self.inner.query_plugin(target, method, args_json)
    }

    fn register_service(&self, name: &str) {
        if !self.check_capability("register_service", "bus.publish") {
            return;
        }
        self.inner.register_service(name)
    }

    fn get_config(&self, key: &str) -> Option<String> {
        if !self.check_capability("get_config", "config.read") {
            return None;
        }
        self.inner.get_config(key)
    }

    fn set_config(&self, key: &str, value: &str) {
        if !self.check_capability("set_config", "config.write") {
            return;
        }
        self.inner.set_config(key, value)
    }

    fn clipboard_set(&self, text: &str) {
        if !self.check_capability("clipboard_set", "clipboard.write") {
            return;
        }
        self.inner.clipboard_set(text)
    }

    fn clipboard_get(&self) -> Option<String> {
        if !self.check_capability("clipboard_get", "clipboard.read") {
            return None;
        }
        self.inner.clipboard_get()
    }

    fn get_theme(&self) -> Option<String> {
        self.inner.get_theme()
    }

    fn register_menu_item(&self, menu: &str, label: &str, action: &str, keybind: Option<&str>) {
        if !self.check_capability("register_menu_item", "ui.menu") {
            return;
        }
        self.inner.register_menu_item(menu, label, action, keybind)
    }

    fn show_form(&self, json: &str) -> Option<String> {
        if !self.check_capability("show_form", "ui.dialog") {
            return None;
        }
        self.inner.show_form(json)
    }

    fn show_confirm(&self, msg: &str) -> bool {
        if !self.check_capability("show_confirm", "ui.dialog") {
            return false;
        }
        self.inner.show_confirm(msg)
    }

    fn show_prompt(&self, msg: &str, default_value: &str) -> Option<String> {
        if !self.check_capability("show_prompt", "ui.dialog") {
            return None;
        }
        self.inner.show_prompt(msg, default_value)
    }

    fn show_alert(&self, title: &str, msg: &str) {
        if !self.check_capability("show_alert", "ui.dialog") {
            return;
        }
        self.inner.show_alert(title, msg)
    }

    fn show_error(&self, title: &str, msg: &str) {
        if !self.check_capability("show_error", "ui.dialog") {
            return;
        }
        self.inner.show_error(title, msg)
    }

    fn show_context_menu(&self, json: &str) -> Option<String> {
        if !self.check_capability("show_context_menu", "ui.dialog") {
            return None;
        }
        self.inner.show_context_menu(json)
    }

    fn write_to_pty(&self, data: &[u8]) {
        if !self.check_capability("write_to_pty", "session.write") {
            return;
        }
        self.inner.write_to_pty(data)
    }

    fn new_tab(&self, command: Option<&str>, plain: bool) {
        if !self.check_capability("new_tab", "session.new_tab") {
            return;
        }
        self.inner.new_tab(command, plain)
    }

    fn open_session(&self, meta_json: &str) -> u64 {
        if !self.check_capability("open_session", "session.open") {
            return 0;
        }
        self.inner.open_session(meta_json)
    }

    fn close_session(&self, handle: u64) {
        if !self.check_capability("close_session", "session.close") {
            return;
        }
        self.inner.close_session(handle)
    }

    fn set_session_status(&self, handle: u64, status: u8, detail: Option<&str>) {
        if !self.check_capability("set_session_status", "session.status") {
            return;
        }
        self.inner.set_session_status(handle, status, detail)
    }

    fn session_prompt(
        &self,
        handle: u64,
        prompt_type: u8,
        msg: &str,
        detail: Option<&str>,
    ) -> Option<String> {
        if !self.check_capability("session_prompt", "session.exec") {
            return None;
        }
        self.inner.session_prompt(handle, prompt_type, msg, detail)
    }
}
