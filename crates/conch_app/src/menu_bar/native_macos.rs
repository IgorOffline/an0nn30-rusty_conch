//! Native macOS menu bar using NSMenu/NSMenuItem via objc2.
//!
//! Installs a global menu bar so the app feels native on macOS.
//! Menu actions are communicated back via a global channel that
//! the app polls each frame.

use std::sync::{LazyLock, Mutex};

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, Sel};
use objc2::{define_class, msg_send, sel, MainThreadOnly};
use objc2_app_kit::{NSApplication, NSMenu, NSMenuItem};
use objc2_foundation::{MainThreadMarker, NSString};

use super::MenuAction;

/// Global channel for menu actions.
static MENU_ACTIONS: LazyLock<Mutex<Vec<MenuAction>>> =
    LazyLock::new(|| Mutex::new(Vec::new()));

fn push_action(action: MenuAction) {
    if let Ok(mut v) = MENU_ACTIONS.lock() {
        v.push(action);
    }
}

pub fn drain_actions() -> Vec<MenuAction> {
    MENU_ACTIONS
        .lock()
        .map(|mut v| std::mem::take(&mut *v))
        .unwrap_or_default()
}

// ── ObjC responder class ──

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "ConchMenuResponder"]
    #[ivars = ()]
    struct MenuResponder;

    impl MenuResponder {
        #[unsafe(method(newTab:))]
        fn new_tab(&self, _sender: *mut AnyObject) {
            push_action(MenuAction::NewTab);
        }

        #[unsafe(method(newWindow:))]
        fn new_window(&self, _sender: *mut AnyObject) {
            push_action(MenuAction::NewWindow);
        }

        #[unsafe(method(closeTab:))]
        fn close_tab(&self, _sender: *mut AnyObject) {
            push_action(MenuAction::CloseTab);
        }

        #[unsafe(method(doCopy:))]
        fn do_copy(&self, _sender: *mut AnyObject) {
            push_action(MenuAction::Copy);
        }

        #[unsafe(method(doPaste:))]
        fn do_paste(&self, _sender: *mut AnyObject) {
            push_action(MenuAction::Paste);
        }

        #[unsafe(method(selectAll:))]
        fn select_all(&self, _sender: *mut AnyObject) {
            push_action(MenuAction::SelectAll);
        }

        #[unsafe(method(zenMode:))]
        fn zen_mode(&self, _sender: *mut AnyObject) {
            push_action(MenuAction::ZenMode);
        }

        #[unsafe(method(zoomIn:))]
        fn zoom_in(&self, _sender: *mut AnyObject) {
            push_action(MenuAction::ZoomIn);
        }

        #[unsafe(method(zoomOut:))]
        fn zoom_out(&self, _sender: *mut AnyObject) {
            push_action(MenuAction::ZoomOut);
        }

        #[unsafe(method(zoomReset:))]
        fn zoom_reset(&self, _sender: *mut AnyObject) {
            push_action(MenuAction::ZoomReset);
        }
    }
);

impl MenuResponder {
    fn create(mtm: MainThreadMarker) -> Retained<Self> {
        let this = mtm.alloc::<Self>().set_ivars(());
        unsafe { msg_send![super(this), init] }
    }
}

/// Global responder — must stay alive for the app's lifetime.
static RESPONDER: LazyLock<Mutex<Option<Retained<MenuResponder>>>> =
    LazyLock::new(|| Mutex::new(None));

/// Set up the native macOS menu bar. Call once from the main thread.
pub fn setup_menu_bar() {
    let mtm = MainThreadMarker::new()
        .expect("setup_menu_bar must be called from the main thread");
    let responder = MenuResponder::create(mtm);

    unsafe {
        let app = NSApplication::sharedApplication(mtm);
        let main_menu = NSMenu::new(mtm);

        // ── App menu (Conch) ──
        let app_menu = NSMenu::new(mtm);
        app_menu.addItem(&make_item_no_target(mtm, "About Conch", sel!(orderFrontStandardAboutPanel:), ""));
        app_menu.addItem(&NSMenuItem::separatorItem(mtm));
        app_menu.addItem(&make_item_no_target(mtm, "Quit Conch", sel!(terminate:), "q"));
        let app_item = NSMenuItem::new(mtm);
        app_item.setSubmenu(Some(&app_menu));
        main_menu.addItem(&app_item);

        // ── File ──
        let file_menu = make_menu(mtm, "File");
        file_menu.addItem(&make_item(mtm, "New Tab", sel!(newTab:), "t", &responder));
        file_menu.addItem(&make_item(mtm, "New Window", sel!(newWindow:), "N", &responder));
        file_menu.addItem(&NSMenuItem::separatorItem(mtm));
        file_menu.addItem(&make_item(mtm, "Close Tab", sel!(closeTab:), "w", &responder));
        let file_item = NSMenuItem::new(mtm);
        file_item.setSubmenu(Some(&file_menu));
        main_menu.addItem(&file_item);

        // ── Edit ──
        let edit_menu = make_menu(mtm, "Edit");
        edit_menu.addItem(&make_item(mtm, "Copy", sel!(doCopy:), "c", &responder));
        edit_menu.addItem(&make_item(mtm, "Paste", sel!(doPaste:), "v", &responder));
        edit_menu.addItem(&NSMenuItem::separatorItem(mtm));
        edit_menu.addItem(&make_item(mtm, "Select All", sel!(selectAll:), "a", &responder));
        let edit_item = NSMenuItem::new(mtm);
        edit_item.setSubmenu(Some(&edit_menu));
        main_menu.addItem(&edit_item);

        // ── View ──
        let view_menu = make_menu(mtm, "View");
        view_menu.addItem(&make_item(mtm, "Zen Mode", sel!(zenMode:), "", &responder));
        view_menu.addItem(&NSMenuItem::separatorItem(mtm));
        view_menu.addItem(&make_item(mtm, "Zoom In", sel!(zoomIn:), "+", &responder));
        view_menu.addItem(&make_item(mtm, "Zoom Out", sel!(zoomOut:), "-", &responder));
        view_menu.addItem(&make_item(mtm, "Reset Zoom", sel!(zoomReset:), "0", &responder));
        let view_item = NSMenuItem::new(mtm);
        view_item.setSubmenu(Some(&view_menu));
        main_menu.addItem(&view_item);

        // ── Help ──
        let help_menu = make_menu(mtm, "Help");
        help_menu.addItem(&make_item_no_target(mtm, "About Conch", sel!(orderFrontStandardAboutPanel:), ""));
        let help_item = NSMenuItem::new(mtm);
        help_item.setSubmenu(Some(&help_menu));
        main_menu.addItem(&help_item);

        app.setMainMenu(Some(&main_menu));
    }

    // Keep responder alive.
    *RESPONDER.lock().unwrap() = Some(responder);
}

unsafe fn make_menu(mtm: MainThreadMarker, title: &str) -> Retained<NSMenu> {
    let ns_title = NSString::from_str(title);
    NSMenu::initWithTitle(NSMenu::alloc(mtm), &ns_title)
}

unsafe fn make_item(
    mtm: MainThreadMarker,
    title: &str,
    action: Sel,
    key_equiv: &str,
    target: &MenuResponder,
) -> Retained<NSMenuItem> {
    let ns_title = NSString::from_str(title);
    let ns_key = NSString::from_str(key_equiv);
    let item = unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(
            NSMenuItem::alloc(mtm),
            &ns_title,
            Some(action),
            &ns_key,
        )
    };
    let target_ptr: *const MenuResponder = target;
    let _: () = msg_send![&*item, setTarget: target_ptr];
    item
}

unsafe fn make_item_no_target(
    mtm: MainThreadMarker,
    title: &str,
    action: Sel,
    key_equiv: &str,
) -> Retained<NSMenuItem> {
    let ns_title = NSString::from_str(title);
    let ns_key = NSString::from_str(key_equiv);
    unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(
            NSMenuItem::alloc(mtm),
            &ns_title,
            Some(action),
            &ns_key,
        )
    }
}
