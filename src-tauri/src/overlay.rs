//! Flow Bar overlay — always-on-top, non-activating pill host.
//!
//! Fixed top-center placement (under the iMac notch / menu bar) on the monitor
//! where the frontmost app lives. Avoids covering chat composers in Codex, Cursor, etc.

use tauri::{
    AppHandle, LogicalPosition, LogicalSize, Manager, Monitor, Position, Size, WebviewUrl,
    WebviewWindow,
};
#[cfg(not(target_os = "macos"))]
use tauri::WebviewWindowBuilder;

pub const OVERLAY_LABEL: &str = "whimpr_bar";

const OVERLAY_W: f64 = 320.0;
const OVERLAY_H: f64 = 80.0;
/// Logical pt below the top of the monitor — sits in the notch divot on iMacs.
const TOP_INSET: f64 = 52.0;

#[cfg(target_os = "macos")]
use tauri_nspanel::ManagerExt;
#[cfg(target_os = "macos")]
use tauri_nspanel::{tauri_panel, CollectionBehavior, PanelBuilder, PanelLevel, StyleMask};

#[cfg(target_os = "macos")]
tauri_panel! {
    panel!(FlowBarPanel {
        config: {
            can_become_key_window: false,
            is_floating_panel: true
        }
    })
}

pub fn build_overlay(app: &tauri::App) -> tauri::Result<WebviewWindow> {
    #[cfg(target_os = "macos")]
    build_macos_panel(app.handle())?;

    #[cfg(not(target_os = "macos"))]
    {
        let overlay = WebviewWindowBuilder::new(
            app,
            OVERLAY_LABEL,
            WebviewUrl::App("overlay.html".into()),
        )
        .title("WhimprBar")
        .inner_size(OVERLAY_W, OVERLAY_H)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .focused(false)
        .focusable(false)
        .accept_first_mouse(true)
        .visible_on_all_workspaces(true)
        .resizable(false)
        .visible(true)
        .build()?;
        present_on(app.handle(), &overlay);
    }

    app.get_webview_window(OVERLAY_LABEL)
        .ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "overlay window not registered after build",
            )
        })
        .map_err(Into::into)
}

#[cfg(target_os = "macos")]
fn build_macos_panel(app: &AppHandle) -> tauri::Result<()> {
    let (x, y) = overlay_position(app, OVERLAY_W, OVERLAY_H).unwrap_or((200.0, 200.0));
    eprintln!("[whimpr] overlay panel at logical ({x:.0}, {y:.0})");

    let panel = PanelBuilder::<_, FlowBarPanel>::new(app, OVERLAY_LABEL)
        .url(WebviewUrl::App("overlay.html".into()))
        .title("WhimprBar")
        .position(Position::Logical(LogicalPosition { x, y }))
        .size(Size::Logical(LogicalSize {
            width: OVERLAY_W,
            height: OVERLAY_H,
        }))
        .level(PanelLevel::Status)
        .has_shadow(false)
        .transparent(true)
        .no_activate(true)
        .corner_radius(0.0)
        .style_mask(StyleMask::empty().borderless().nonactivating_panel())
        .with_window(|w| {
            w.decorations(false)
                .transparent(true)
                .shadow(false)
                .always_on_top(true)
                .skip_taskbar(true)
                .focused(false)
                .focusable(false)
                .accept_first_mouse(true)
                .visible_on_all_workspaces(true)
                .resizable(false)
        })
        .collection_behavior(
            CollectionBehavior::new()
                .can_join_all_spaces()
                .full_screen_auxiliary()
                .ignores_cycle(),
        )
        .build()
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;

    panel.show();
    Ok(())
}

/// Re-anchor and re-assert z-order before every pill state change.
pub fn present(app: &AppHandle) {
    let app = app.clone();
    let app2 = app.clone();
    if let Err(e) = app.run_on_main_thread(move || {
        if let Some(w) = app2.get_webview_window(OVERLAY_LABEL) {
            present_on(&app2, &w);
        } else {
            eprintln!("[whimpr] overlay window missing (label={OVERLAY_LABEL})");
        }
    }) {
        eprintln!("[whimpr] overlay present failed to schedule on main thread: {e}");
    }
}

fn present_on(app: &AppHandle, w: &WebviewWindow) {
    if let Some((x, y)) = overlay_position(app, OVERLAY_W, OVERLAY_H) {
        let _ = w.set_position(Position::Logical(LogicalPosition { x, y }));
    }
    let _ = w.set_always_on_top(true);
    let _ = w.set_visible_on_all_workspaces(true);
    let _ = w.show();

    #[cfg(target_os = "macos")]
    if let Ok(panel) = app.get_webview_panel(OVERLAY_LABEL) {
        panel.show();
    }
}

fn monitor_containing_point(app: &AppHandle, lx: f64, ly: f64) -> Option<Monitor> {
    let monitors = app.available_monitors().ok()?;
    for m in monitors {
        let scale = m.scale_factor();
        let mx = m.position().x as f64 / scale;
        let my = m.position().y as f64 / scale;
        let mw = m.size().width as f64 / scale;
        let mh = m.size().height as f64 / scale;
        if lx >= mx && lx < mx + mw && ly >= my && ly < my + mh {
            return Some(m);
        }
    }
    None
}

/// Monitor where the user is working — follows the frontmost app across displays.
fn active_monitor(app: &AppHandle) -> Option<Monitor> {
    if let Some(win) = crate::caret::frontmost_window_frame() {
        let (cx, cy) = crate::caret::rect_center(win);
        if let Some(m) = monitor_containing_point(app, cx, cy) {
            return Some(m);
        }
    }
    app.cursor_position()
        .ok()
        .and_then(|pos| app.monitor_from_point(pos.x, pos.y).ok().flatten())
        .or_else(|| {
            app.get_webview_window(OVERLAY_LABEL)
                .and_then(|w| w.primary_monitor().ok().flatten())
        })
}

/// Top-center of `monitor`, inset below the menu bar / notch divot.
fn top_center_on_monitor(monitor: &Monitor, width: f64) -> (f64, f64) {
    let scale = monitor.scale_factor();
    let monitor_x = monitor.position().x as f64 / scale;
    let monitor_y = monitor.position().y as f64 / scale;
    let monitor_width = monitor.size().width as f64 / scale;
    let x = monitor_x + (monitor_width - width) / 2.0;
    let y = monitor_y + TOP_INSET;
    (x, y)
}

fn overlay_position(app: &AppHandle, width: f64, _height: f64) -> Option<(f64, f64)> {
    let monitor = active_monitor(app)?;
    let (x, y) = top_center_on_monitor(&monitor, width);
    Some((x, y))
}
