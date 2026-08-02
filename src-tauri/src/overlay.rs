//! Flow Bar overlay — NSPanel on macOS for visibility on multi-monitor Retina setups.
//!
//! Tauri events do not reliably reach panel webviews, so Rust keeps a live snapshot
//! the overlay polls via `get_overlay_snapshot`. Events are still emitted as a backup.

use serde::Serialize;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{
    AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, Monitor, Position, Size,
    WebviewUrl, WebviewWindow,
};
#[cfg(not(target_os = "macos"))]
use tauri::WebviewWindowBuilder;

pub const OVERLAY_LABEL: &str = "whimpr_bar";
pub const EVENT_STATE: &str = "whimpr://flowbar/state";
pub const EVENT_WAVEFORM: &str = "whimpr://audio/waveform";

const OVERLAY_W: f64 = 320.0;
const OVERLAY_H: f64 = 80.0;
/// Logical pt below the top of the monitor — sits in the notch divot on iMacs.
const TOP_INSET: f64 = 52.0;
const MIC_EMIT_THROTTLE_MS: u64 = 33;

static LAST_MIC_EMIT_MS: AtomicU64 = AtomicU64::new(0);

/// Latest pill UI — polled by the overlay webview (reliable with NSPanel).
#[derive(Clone, Serialize, Default)]
pub struct OverlaySnapshot {
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    pub bars: Vec<f32>,
}

static SNAPSHOT: OnceLock<Mutex<OverlaySnapshot>> = OnceLock::new();

fn snapshot_store() -> &'static Mutex<OverlaySnapshot> {
    SNAPSHOT.get_or_init(|| {
        Mutex::new(OverlaySnapshot {
            state: "idle".into(),
            message: None,
            bars: Vec::new(),
        })
    })
}

#[tauri::command]
pub fn get_overlay_snapshot() -> OverlaySnapshot {
    snapshot_store().lock().unwrap().clone()
}

pub fn push_state(state: &str, message: Option<String>) {
    {
        let mut snap = snapshot_store().lock().unwrap();
        snap.state = state.to_string();
        snap.message = message;
    }
}

pub fn push_waveform(bars: &[f32]) {
    snapshot_store().lock().unwrap().bars = bars.to_vec();
}

pub fn emit_state(app: &AppHandle, state: &str, message: Option<String>) {
    push_state(state, message.clone());
    let _ = app.emit_to(
        OVERLAY_LABEL,
        EVENT_STATE,
        StatePayload {
            state,
            message,
        },
    );
    present(app);
}

pub fn emit_waveform(app: &AppHandle, bars: &[f32]) {
    push_waveform(bars);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let last = LAST_MIC_EMIT_MS.load(Ordering::Relaxed);
    if now.saturating_sub(last) < MIC_EMIT_THROTTLE_MS {
        return;
    }
    LAST_MIC_EMIT_MS.store(now, Ordering::Relaxed);
    let _ = app.emit_to(
        OVERLAY_LABEL,
        EVENT_WAVEFORM,
        WavePayload {
            bars: bars.to_vec(),
        },
    );
}

#[derive(Clone, Serialize)]
struct StatePayload<'a> {
    state: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
}

#[derive(Clone, Serialize)]
struct WavePayload {
    bars: Vec<f32>,
}

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
