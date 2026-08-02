//! Flow Bar overlay — always-on-top, non-activating pill host.
//!
//! Fixed top-center placement on the computer's primary display.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;
#[cfg(not(target_os = "macos"))]
use tauri::WebviewWindowBuilder;
use tauri::{
    AppHandle, LogicalPosition, LogicalSize, Manager, Monitor, Position, Size, WebviewUrl,
    WebviewWindow,
};
#[cfg(target_os = "macos")]
use tauri_nspanel::{tauri_panel, ManagerExt, PanelBuilder, PanelLevel, StyleMask};

pub const OVERLAY_LABEL: &str = "whimpr_bar";

const OVERLAY_W: f64 = 320.0;
const OVERLAY_H: f64 = 80.0;
/// Logical pt below the top of the monitor — sits in the notch divot on iMacs.
const TOP_INSET: f64 = 52.0;
const FOLLOW_INTERVAL: Duration = Duration::from_millis(250);

static FOLLOW_STARTED: AtomicBool = AtomicBool::new(false);
static VISIBLE: AtomicBool = AtomicBool::new(true);
static LAST_TARGET_MONITOR: OnceLock<Mutex<Option<(i32, i32)>>> = OnceLock::new();
static LAST_FRONTMOST_FRAME: OnceLock<Mutex<Option<(i32, i32, i32, i32)>>> = OnceLock::new();

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
        let overlay =
            WebviewWindowBuilder::new(app, OVERLAY_LABEL, WebviewUrl::App("overlay.html".into()))
                .title("WhimprBar")
                .inner_size(OVERLAY_W, OVERLAY_H)
                .decorations(false)
                .transparent(true)
                .shadow(false)
                .always_on_top(true)
                .skip_taskbar(true)
                .focused(false)
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
        .with_window(|window| {
            window
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
        })
        .build()
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::Other, error))?;
    configure_panel(&panel);
    panel.order_front_regardless();
    Ok(())
}

/// Re-anchor and re-assert z-order before every pill state change.
pub fn present(app: &AppHandle) {
    if !VISIBLE.load(Ordering::SeqCst) {
        return;
    }
    let app = app.clone();
    let app2 = app.clone();
    if let Err(e) = app.run_on_main_thread(move || {
        if let Some(w) = overlay_window(&app2) {
            present_on(&app2, &w);
        } else {
            eprintln!("[whimpr] overlay window missing (label={OVERLAY_LABEL})");
        }
    }) {
        eprintln!("[whimpr] overlay present failed to schedule on main thread: {e}");
    }
}

pub fn is_enabled() -> bool {
    VISIBLE.load(Ordering::SeqCst)
}

pub fn set_enabled(app: &AppHandle, enabled: bool) {
    VISIBLE.store(enabled, Ordering::SeqCst);
    let app = app.clone();
    let app_for_task = app.clone();
    if let Err(error) = app.run_on_main_thread(move || {
        let Some(window) = overlay_window(&app_for_task) else {
            return;
        };
        if enabled {
            present_on(&app_for_task, &window);
        } else {
            let _ = window.hide();
        }
    }) {
        eprintln!("[whimpr] overlay visibility scheduling failed: {error}");
    }
}

/// Keep the pill anchored to the display containing the frontmost app. This
/// runs independently of dictation state so moving between displays while idle
/// is reflected before the next recording starts.
pub fn start_monitor_follow(app: AppHandle) {
    if FOLLOW_STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    if let Ok(monitors) = app.available_monitors() {
        for (index, monitor) in monitors.iter().enumerate() {
            eprintln!(
                "[whimpr] display[{index}] name={:?} pos=({}, {}) size={}x{} scale={}",
                monitor.name(),
                monitor.position().x,
                monitor.position().y,
                monitor.size().width,
                monitor.size().height,
                monitor.scale_factor()
            );
        }
    }
    std::thread::spawn(move || loop {
        std::thread::sleep(FOLLOW_INTERVAL);
        let app_for_task = app.clone();
        if app
            .run_on_main_thread(move || {
                if VISIBLE.load(Ordering::SeqCst) {
                    if let Some(window) = overlay_window(&app_for_task) {
                        reposition_on_active_monitor(&app_for_task, &window);
                    }
                }
            })
            .is_err()
        {
            break;
        }
    });
}

pub fn dispatch_state(app: &AppHandle, state: &str, message: Option<String>) {
    dispatch_to_panel(
        app,
        "__WHIMPR_OVERLAY_STATE__",
        "whimpr:overlay-state",
        serde_json::json!({ "state": state, "message": message }),
    );
}

pub fn dispatch_waveform(app: &AppHandle, bars: &[f32]) {
    dispatch_to_panel(
        app,
        "__WHIMPR_OVERLAY_WAVEFORM__",
        "whimpr:overlay-waveform",
        serde_json::json!({ "bars": bars }),
    );
}

pub fn dispatch_partial(app: &AppHandle, text: &str) {
    dispatch_to_panel(
        app,
        "__WHIMPR_OVERLAY_PARTIAL__",
        "whimpr:overlay-partial",
        serde_json::json!({ "text": text }),
    );
}

/// WebKit evaluation must happen on the macOS main thread. This bypasses the
/// NSPanel event-channel limitation without sacrificing the panel's visibility.
fn dispatch_to_panel(app: &AppHandle, global: &str, event: &str, payload: serde_json::Value) {
    let (Ok(global), Ok(event)) = (serde_json::to_string(global), serde_json::to_string(event))
    else {
        return;
    };
    let script = format!(
        "window[{global}] = {payload}; \
         window.dispatchEvent(new CustomEvent({event}, {{ detail: {payload} }}));"
    );
    let app = app.clone();
    let app_for_task = app.clone();
    if let Err(error) = app.run_on_main_thread(move || {
        let Some(window) = overlay_window(&app_for_task) else {
            eprintln!("[whimpr] overlay dispatch failed: window missing");
            return;
        };
        if let Err(error) = window.eval(&script) {
            eprintln!("[whimpr] overlay dispatch failed: {error}");
        }
    }) {
        eprintln!("[whimpr] overlay dispatch scheduling failed: {error}");
    }
}

fn overlay_window(app: &AppHandle) -> Option<WebviewWindow> {
    app.get_webview_window(OVERLAY_LABEL)
}

fn present_on(app: &AppHandle, w: &WebviewWindow) {
    reposition_on_active_monitor(app, w);
    let _ = w.set_visible_on_all_workspaces(true);

    #[cfg(target_os = "macos")]
    if let Ok(panel) = app.get_webview_panel(OVERLAY_LABEL) {
        configure_panel(&panel);
        panel.order_front_regardless();
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = w.set_always_on_top(true);
        let _ = w.show();
    }
}

fn reposition_on_active_monitor(app: &AppHandle, window: &WebviewWindow) {
    let Some(monitor) = active_monitor(app) else {
        return;
    };
    let key = (monitor.position().x, monitor.position().y);
    let target = LAST_TARGET_MONITOR.get_or_init(|| Mutex::new(None));
    let mut previous = target.lock().unwrap();
    if *previous != Some(key) {
        eprintln!(
            "[whimpr] pill monitor -> {:?} @({}, {})",
            monitor.name(),
            key.0,
            key.1
        );
        *previous = Some(key);
    }
    drop(previous);

    let (x, y) = top_center_on_monitor(&monitor, OVERLAY_W);
    let _ = window.set_position(Position::Logical(LogicalPosition { x, y }));
}

#[cfg(target_os = "macos")]
fn configure_panel(panel: &tauri_nspanel::PanelHandle<tauri::Wry>) {
    use objc2_app_kit::{NSStatusWindowLevel, NSWindowCollectionBehavior};

    panel.set_collection_behavior(
        NSWindowCollectionBehavior::CanJoinAllSpaces
            | NSWindowCollectionBehavior::FullScreenAuxiliary
            | NSWindowCollectionBehavior::IgnoresCycle,
    );
    panel.set_level(NSStatusWindowLevel as i64);
    panel.set_floating_panel(true);
}

fn monitor_containing_logical_point(app: &AppHandle, x: f64, y: f64) -> Option<Monitor> {
    app.available_monitors().ok()?.into_iter().find(|monitor| {
        let scale = monitor.scale_factor();
        let origin_x = monitor.position().x as f64 / scale;
        let origin_y = monitor.position().y as f64 / scale;
        let width = monitor.size().width as f64 / scale;
        let height = monitor.size().height as f64 / scale;
        x >= origin_x && x < origin_x + width && y >= origin_y && y < origin_y + height
    })
}

/// Prefer the frontmost app's window, then the cursor, then the primary display.
fn active_monitor(app: &AppHandle) -> Option<Monitor> {
    if let Some(frame) = crate::caret::frontmost_window_frame() {
        let frame_key = (
            frame.x.round() as i32,
            frame.y.round() as i32,
            frame.width.round() as i32,
            frame.height.round() as i32,
        );
        let previous = LAST_FRONTMOST_FRAME.get_or_init(|| Mutex::new(None));
        let mut previous = previous.lock().unwrap();
        if *previous != Some(frame_key) {
            eprintln!(
                "[whimpr] frontmost frame -> ({}, {}) {}x{}",
                frame_key.0, frame_key.1, frame_key.2, frame_key.3
            );
            *previous = Some(frame_key);
        }
        drop(previous);
        let (x, y) = crate::caret::rect_center(frame);
        if let Some(monitor) = monitor_containing_logical_point(app, x, y) {
            return Some(monitor);
        }
    }

    app.cursor_position()
        .ok()
        .and_then(|position| {
            app.monitor_from_point(position.x, position.y)
                .ok()
                .flatten()
        })
        .or_else(|| app.primary_monitor().ok().flatten())
        .or_else(|| app.available_monitors().ok()?.into_iter().next())
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
