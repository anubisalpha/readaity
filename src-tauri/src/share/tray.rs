//! System-tray "Sharing on" indicator. The tray icon exists only while the
//! share server is running; its tooltip shows the peer name + book count and
//! its menu offers Open / Stop sharing.

use std::sync::Mutex;

use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{TrayIcon, TrayIconBuilder},
    AppHandle, Manager,
};

/// Managed state: the live tray icon, if sharing is on.
#[derive(Default)]
pub struct TrayState(pub Mutex<Option<TrayIcon>>);

fn book_count(app: &AppHandle) -> i64 {
    app.try_state::<crate::db::AppDb>()
        .and_then(|dbs| {
            dbs.0.lock().ok().map(|conn| {
                crate::db::ready_count(&conn, "comics").unwrap_or(0)
                    + crate::db::ready_count(&conn, "ebooks").unwrap_or(0)
            })
        })
        .unwrap_or(0)
}

/// Sync the tray icon to the current share state. Safe to call repeatedly.
pub fn refresh(app: &AppHandle) {
    let running = super::status(app).running;
    let state = app.state::<TrayState>();
    let mut slot = match state.0.lock() {
        Ok(s) => s,
        Err(_) => return,
    };

    if !running {
        *slot = None; // dropping the TrayIcon removes it
        return;
    }
    if slot.is_some() {
        // Already shown — just refresh the tooltip (name / count may have moved).
        if let Some(tray) = slot.as_ref() {
            let _ = tray.set_tooltip(Some(tooltip(app)));
        }
        return;
    }

    let Some(icon) = app.default_window_icon().cloned() else {
        return;
    };
    let open = MenuItem::with_id(app, "share_open", "Open Readaity", true, None::<&str>);
    let stop = MenuItem::with_id(app, "share_stop", "Stop sharing", true, None::<&str>);
    let (open, stop) = match (open, stop) {
        (Ok(o), Ok(s)) => (o, s),
        _ => return,
    };
    let sep = match PredefinedMenuItem::separator(app) {
        Ok(s) => s,
        Err(_) => return,
    };
    let menu = match Menu::with_items(app, &[&open, &sep, &stop]) {
        Ok(m) => m,
        Err(_) => return,
    };

    let tray = TrayIconBuilder::with_id("readaity-share")
        .icon(icon)
        .tooltip(tooltip(app))
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "share_open" => {
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.show();
                    let _ = w.unminimize();
                    let _ = w.set_focus();
                }
            }
            "share_stop" => {
                let _ = super::stop(app);
            }
            _ => {}
        })
        .build(app);

    if let Ok(tray) = tray {
        *slot = Some(tray);
    }
}

fn tooltip(app: &AppHandle) -> String {
    let cfg = super::load_config(app);
    format!("Readaity — sharing “{}” ({} books)", cfg.name, book_count(app))
}
